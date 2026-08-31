use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt, fs,
    io::{Cursor, Read},
    net::SocketAddr,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use verge_config::{FileProfileStore, ProfileUpdateJob, UpdateScheduler, UpdateTrigger};
use verge_core::{
    ControllerTransport, CoreSupervisor, MihomoClient, RealtimeOptions, RealtimeSubscription,
    SidecarTarget, install_verified_artifact,
};
use verge_domain::{
    AppCommand, AppCommandOutput, AppCommandResult, AppError, CommandActor, CommandContext,
    CommandRisk, ErrorCode, NetworkSettings, Profile, ProfileId, ProviderKind, ProviderSummary,
    ProxyEndpoint, ProxyGroup, RealtimeEvent, RealtimeTopic, RuleEntry, RunMode, RuntimeCommand,
    RuntimeCommandOutput, RuntimeCommandResult, SystemProxyCommand, SystemProxyCommandResult,
    SystemProxyState,
};
use verge_platform::SystemProxyPlatform;

#[derive(Clone, Copy, Debug, Default)]
pub struct CommandPolicy;

impl CommandPolicy {
    pub fn authorize(&self, context: CommandContext, risk: CommandRisk) -> Result<(), AppError> {
        let implicitly_allowed = match context.actor {
            CommandActor::UserInterface | CommandActor::Tray => {
                !matches!(risk, CommandRisk::Destructive)
            }
            CommandActor::Agent => matches!(risk, CommandRisk::ReadOnly),
            CommandActor::Scheduler => {
                matches!(risk, CommandRisk::ReadOnly | CommandRisk::LowRiskWrite)
            }
        };
        if implicitly_allowed
            || context
                .approval
                .is_some_and(|approval| risk_allowed(approval.max_risk, risk))
        {
            return Ok(());
        }
        Err(AppError::new(
            ErrorCode::PermissionDenied,
            format!(
                "{:?} command requires an explicit approval for {:?}",
                context.actor, risk
            ),
        ))
    }
}

fn risk_allowed(granted: CommandRisk, requested: CommandRisk) -> bool {
    let rank = |risk| match risk {
        CommandRisk::ReadOnly => 0,
        CommandRisk::LowRiskWrite => 1,
        CommandRisk::PrivilegedWrite => 2,
        CommandRisk::Destructive => 3,
    };
    rank(granted) >= rank(requested)
}

#[derive(Default)]
pub struct CommandBus {
    policy: CommandPolicy,
    failed_operations: HashSet<u64>,
}

impl CommandBus {
    pub fn authorize(
        &self,
        operation_id: u64,
        context: CommandContext,
        risk: CommandRisk,
    ) -> Result<(), AppError> {
        if self.failed_operations.contains(&operation_id) {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "operation aborted because an earlier command failed",
            ));
        }
        self.policy.authorize(context, risk)
    }

    pub fn complete(
        &mut self,
        operation_id: u64,
        risk: CommandRisk,
        succeeded: bool,
        last_in_operation: bool,
    ) {
        if !succeeded && !matches!(risk, CommandRisk::ReadOnly) {
            self.failed_operations.insert(operation_id);
        }
        if last_in_operation {
            self.failed_operations.remove(&operation_id);
        }
    }
}

#[cfg(test)]
mod command_bus_tests {
    use super::*;
    use verge_domain::{CommandApproval, CommandContext};

    #[test]
    fn destructive_commands_require_explicit_user_approval() {
        let policy = CommandPolicy;
        let context = CommandContext {
            actor: CommandActor::UserInterface,
            approval: None,
        };
        assert_eq!(
            policy
                .authorize(context, CommandRisk::Destructive)
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        policy
            .authorize(
                CommandContext {
                    approval: Some(CommandApproval {
                        max_risk: CommandRisk::Destructive,
                    }),
                    ..context
                },
                CommandRisk::Destructive,
            )
            .unwrap();
    }

    #[test]
    fn agent_writes_require_a_grant_and_failed_batches_short_circuit() {
        let mut bus = CommandBus::default();
        let agent = CommandContext {
            actor: CommandActor::Agent,
            approval: None,
        };
        assert_eq!(
            bus.authorize(7, agent, CommandRisk::LowRiskWrite)
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        bus.complete(7, CommandRisk::LowRiskWrite, false, false);
        assert_eq!(
            bus.authorize(7, agent, CommandRisk::ReadOnly)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        bus.complete(7, CommandRisk::ReadOnly, false, true);
        bus.authorize(7, agent, CommandRisk::ReadOnly).unwrap();
    }
}

pub trait SystemProxyControl {
    fn recovery_pending(&self) -> bool;
    fn state(&mut self) -> Result<SystemProxyState, AppError>;
    fn enable(
        &mut self,
        services: &[String],
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError>;
    fn disable(&mut self) -> Result<SystemProxyState, AppError>;
    fn recover_pending(&mut self) -> Result<SystemProxyState, AppError>;
    fn set_socks(
        &mut self,
        enabled: bool,
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError>;
    fn set_auto_proxy(&mut self, url: Option<&str>) -> Result<SystemProxyState, AppError>;
    fn set_proxy_bypass(&mut self, domains: &[String]) -> Result<SystemProxyState, AppError>;
}

pub struct PlatformSystemProxy<P> {
    platform: P,
    services: Vec<String>,
}

impl<P: SystemProxyPlatform> PlatformSystemProxy<P> {
    pub fn new(platform: P, services: Vec<String>) -> Result<Self, AppError> {
        if services.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "at least one system proxy service is required",
            ));
        }
        Ok(Self { platform, services })
    }

    pub fn managed_services(&self) -> &[String] {
        &self.services
    }
}

impl<P: SystemProxyPlatform> SystemProxyControl for PlatformSystemProxy<P> {
    fn recovery_pending(&self) -> bool {
        self.platform.recovery_pending()
    }

    fn state(&mut self) -> Result<SystemProxyState, AppError> {
        self.platform.state(&self.services)
    }

    fn enable(
        &mut self,
        services: &[String],
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError> {
        let services = if services.is_empty() {
            &self.services
        } else {
            services
        };
        let state = self.platform.enable(services, endpoint)?;
        self.services = services.to_vec();
        Ok(state)
    }

    fn disable(&mut self) -> Result<SystemProxyState, AppError> {
        self.platform.disable()
    }

    fn recover_pending(&mut self) -> Result<SystemProxyState, AppError> {
        self.platform.recover_pending()
    }

    fn set_socks(
        &mut self,
        enabled: bool,
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError> {
        self.platform.set_socks(&self.services, enabled, endpoint)
    }

    fn set_auto_proxy(&mut self, url: Option<&str>) -> Result<SystemProxyState, AppError> {
        self.platform.set_auto_proxy(&self.services, url)
    }

    fn set_proxy_bypass(&mut self, domains: &[String]) -> Result<SystemProxyState, AppError> {
        self.platform.set_bypass(&self.services, domains)
    }
}

pub struct SystemProxyCommandHandler<'a, C> {
    proxy: &'a mut C,
}

impl<'a, C: SystemProxyControl> SystemProxyCommandHandler<'a, C> {
    pub fn new(proxy: &'a mut C) -> Self {
        Self { proxy }
    }

    pub fn execute(
        &mut self,
        command: SystemProxyCommand,
    ) -> Result<SystemProxyCommandResult, AppError> {
        let (state, summary) = match command {
            SystemProxyCommand::GetState => (self.proxy.state()?, "System proxy state loaded"),
            SystemProxyCommand::Enable { services, endpoint } => (
                self.proxy.enable(&services, &endpoint)?,
                "System proxy enabled",
            ),
            SystemProxyCommand::Disable => (self.proxy.disable()?, "System proxy restored"),
            SystemProxyCommand::RecoverPending => (
                self.proxy.recover_pending()?,
                "Pending system proxy recovery applied",
            ),
            SystemProxyCommand::SetSocks { enabled, endpoint } => (
                self.proxy.set_socks(enabled, &endpoint)?,
                "SOCKS proxy settings updated",
            ),
            SystemProxyCommand::SetAutoProxy { url } => (
                self.proxy.set_auto_proxy(url.as_deref())?,
                "Automatic proxy settings updated",
            ),
            SystemProxyCommand::SetProxyBypass { domains } => (
                self.proxy.set_proxy_bypass(&domains)?,
                "Proxy bypass domains updated",
            ),
        };
        Ok(SystemProxyCommandResult {
            state,
            summary: summary.into(),
        })
    }
}

pub trait RuntimeControl {
    fn mode(&mut self) -> Result<RunMode, AppError>;
    fn set_mode(&mut self, mode: RunMode) -> Result<(), AppError>;
    fn proxy_groups(&mut self) -> Result<Vec<ProxyGroup>, AppError>;
    fn rules(&mut self) -> Result<Vec<RuleEntry>, AppError>;
    fn providers(&mut self) -> Result<Vec<ProviderSummary>, AppError>;
    fn update_provider(&mut self, kind: ProviderKind, name: &str) -> Result<(), AppError>;
    fn network_settings(&mut self) -> Result<NetworkSettings, AppError>;
    fn set_network_settings(&mut self, settings: &NetworkSettings) -> Result<(), AppError>;
    fn select_proxy(&mut self, group: &str, proxy: &str) -> Result<(), AppError>;
    fn delay(&mut self, proxy: &str, url: &str, timeout_ms: u32) -> Result<u32, AppError>;
    fn close_connection(&mut self, id: &str) -> Result<(), AppError>;
    fn start_realtime(&mut self, topics: &[RealtimeTopic]) -> Result<(), AppError>;
    fn stop_realtime(&mut self);
    fn drain_realtime(&mut self) -> Vec<RealtimeEvent>;
}

pub struct MihomoRuntime<T> {
    client: MihomoClient<T>,
    controller: SocketAddr,
    secret: String,
    realtime_options: RealtimeOptions,
    subscriptions: HashMap<RealtimeTopic, RealtimeSubscription>,
    sensitive_values: Vec<String>,
}

impl<T: ControllerTransport> MihomoRuntime<T> {
    pub fn new(
        client: MihomoClient<T>,
        controller: SocketAddr,
        secret: impl Into<String>,
        realtime_options: RealtimeOptions,
    ) -> Result<Self, AppError> {
        if !controller.ip().is_loopback() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Mihomo runtime controller must use a loopback address",
            ));
        }
        let secret = secret.into();
        if secret.is_empty() || secret.chars().any(char::is_control) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Mihomo runtime secret is empty or contains control characters",
            ));
        }
        Ok(Self {
            client,
            controller,
            secret,
            realtime_options,
            subscriptions: HashMap::new(),
            sensitive_values: std::env::var("HOME").into_iter().collect(),
        })
    }

    pub fn set_sensitive_values(&mut self, values: impl IntoIterator<Item = String>) {
        self.sensitive_values = std::env::var("HOME")
            .into_iter()
            .chain(values)
            .filter(|value| !value.is_empty())
            .collect();
        self.sensitive_values
            .sort_by_key(|value| std::cmp::Reverse(value.len()));
        self.sensitive_values.dedup();
    }
}

impl<T: ControllerTransport> RuntimeControl for MihomoRuntime<T> {
    fn mode(&mut self) -> Result<RunMode, AppError> {
        self.client.mode()
    }

    fn set_mode(&mut self, mode: RunMode) -> Result<(), AppError> {
        self.client.set_mode(mode)
    }

    fn proxy_groups(&mut self) -> Result<Vec<ProxyGroup>, AppError> {
        self.client.proxy_groups()
    }

    fn rules(&mut self) -> Result<Vec<RuleEntry>, AppError> {
        self.client.rules()
    }

    fn providers(&mut self) -> Result<Vec<ProviderSummary>, AppError> {
        self.client.providers()
    }

    fn update_provider(&mut self, kind: ProviderKind, name: &str) -> Result<(), AppError> {
        self.client.update_provider(kind, name)
    }

    fn network_settings(&mut self) -> Result<NetworkSettings, AppError> {
        self.client.network_settings()
    }

    fn set_network_settings(&mut self, settings: &NetworkSettings) -> Result<(), AppError> {
        self.client.set_network_settings(settings)
    }

    fn select_proxy(&mut self, group: &str, proxy: &str) -> Result<(), AppError> {
        self.client.select_proxy(group, proxy)
    }

    fn delay(&mut self, proxy: &str, url: &str, timeout_ms: u32) -> Result<u32, AppError> {
        self.client.delay(proxy, url, timeout_ms)
    }

    fn close_connection(&mut self, id: &str) -> Result<(), AppError> {
        self.client.close_connection(id)
    }

    fn start_realtime(&mut self, topics: &[RealtimeTopic]) -> Result<(), AppError> {
        let mut started = Vec::new();
        for topic in topics.iter().copied() {
            if self.subscriptions.contains_key(&topic) {
                continue;
            }
            match RealtimeSubscription::spawn(
                self.controller,
                &self.secret,
                topic,
                self.realtime_options.clone(),
            ) {
                Ok(subscription) => {
                    self.subscriptions.insert(topic, subscription);
                    started.push(topic);
                }
                Err(error) => {
                    for topic in started {
                        if let Some(mut subscription) = self.subscriptions.remove(&topic) {
                            subscription.stop();
                        }
                    }
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn stop_realtime(&mut self) {
        for (_, mut subscription) in self.subscriptions.drain() {
            subscription.stop();
        }
    }

    fn drain_realtime(&mut self) -> Vec<RealtimeEvent> {
        self.subscriptions
            .values()
            .flat_map(RealtimeSubscription::drain)
            .map(|event| redact_realtime_event(event, &self.secret, &self.sensitive_values))
            .collect()
    }
}

fn redact_realtime_event(
    event: RealtimeEvent,
    secret: &str,
    sensitive_values: &[String],
) -> RealtimeEvent {
    let RealtimeEvent::Log(mut log) = event else {
        return event;
    };
    let mut payload = log.payload.replace(secret, "[REDACTED_SECRET]");
    for value in sensitive_values {
        payload = payload.replace(value, "[REDACTED]");
    }
    for marker in ["Authorization: Bearer ", "Proxy-Authorization: "] {
        if let Some(start) = payload.find(marker) {
            let value_start = start + marker.len();
            let value_end = payload[value_start..]
                .find(char::is_whitespace)
                .map_or(payload.len(), |offset| value_start + offset);
            payload.replace_range(value_start..value_end, "[REDACTED]");
        }
    }
    log.payload = payload;
    RealtimeEvent::Log(log)
}

impl<T> Drop for MihomoRuntime<T> {
    fn drop(&mut self) {
        for (_, mut subscription) in self.subscriptions.drain() {
            subscription.stop();
        }
    }
}

pub struct RuntimeCommandHandler<'a, C> {
    runtime: &'a mut C,
}

impl<'a, C: RuntimeControl> RuntimeCommandHandler<'a, C> {
    pub fn new(runtime: &'a mut C) -> Self {
        Self { runtime }
    }

    pub fn execute(&mut self, command: RuntimeCommand) -> Result<RuntimeCommandResult, AppError> {
        let (output, summary) = match command {
            RuntimeCommand::GetMode => (
                RuntimeCommandOutput::Mode(self.runtime.mode()?),
                "Current mode loaded",
            ),
            RuntimeCommand::SetMode { mode } => {
                self.runtime.set_mode(mode)?;
                (RuntimeCommandOutput::Mode(mode), "Mode changed")
            }
            RuntimeCommand::ListProxyGroups => (
                RuntimeCommandOutput::ProxyGroups(self.runtime.proxy_groups()?),
                "Proxy groups loaded",
            ),
            RuntimeCommand::ListRules => (
                RuntimeCommandOutput::Rules(self.runtime.rules()?),
                "Rules loaded",
            ),
            RuntimeCommand::ListProviders => (
                RuntimeCommandOutput::Providers(self.runtime.providers()?),
                "Providers loaded",
            ),
            RuntimeCommand::UpdateProvider { kind, name } => {
                self.runtime.update_provider(kind, &name)?;
                (RuntimeCommandOutput::None, "Provider updated")
            }
            RuntimeCommand::GetNetworkSettings => (
                RuntimeCommandOutput::NetworkSettings(self.runtime.network_settings()?),
                "Network settings loaded",
            ),
            RuntimeCommand::SetNetworkSettings { settings } => {
                self.runtime.set_network_settings(&settings)?;
                (
                    RuntimeCommandOutput::NetworkSettings(settings),
                    "Network settings changed",
                )
            }
            RuntimeCommand::SelectProxy { group, proxy } => {
                self.runtime.select_proxy(&group, &proxy)?;
                (RuntimeCommandOutput::None, "Proxy selected")
            }
            RuntimeCommand::TestProxyDelay {
                proxy,
                url,
                timeout_ms,
            } => (
                RuntimeCommandOutput::Delay(self.runtime.delay(&proxy, &url, timeout_ms)?),
                "Proxy delay tested",
            ),
            RuntimeCommand::CloseConnection { id } => {
                self.runtime.close_connection(&id)?;
                (RuntimeCommandOutput::None, "Connection closed")
            }
            RuntimeCommand::StartRealtime { topics } => {
                self.runtime.start_realtime(&topics)?;
                (RuntimeCommandOutput::None, "Realtime subscriptions started")
            }
            RuntimeCommand::StopRealtime => {
                self.runtime.stop_realtime();
                (RuntimeCommandOutput::None, "Realtime subscriptions stopped")
            }
            RuntimeCommand::DrainRealtime => (
                RuntimeCommandOutput::Realtime(self.runtime.drain_realtime()),
                "Realtime events drained",
            ),
        };
        Ok(RuntimeCommandResult {
            output,
            summary: summary.into(),
        })
    }
}

pub trait CoreControl {
    fn validate_candidate(&mut self, path: &Path) -> Result<(), AppError>;
    fn apply_config(&mut self, path: &Path) -> Result<(), AppError>;
    fn start(&mut self) -> Result<(), AppError>;
    fn health(&mut self) -> Result<(), AppError>;
    fn stop(&mut self) -> Result<(), AppError>;
}

pub fn update_mihomo_artifact<C: CoreControl>(
    core: &mut C,
    candidate: &Path,
    active: &Path,
    expected_sha256: &str,
) -> Result<(), CommandFailure> {
    core.stop().map_err(no_recovery)?;
    let install = match install_verified_artifact(candidate, active, expected_sha256) {
        Ok(install) => install,
        Err(cause) => {
            let recovery = restart_core(core);
            return Err(CommandFailure { cause, recovery });
        }
    };

    if let Err(cause) = core.start().and_then(|()| core.health()) {
        let mut recovery_errors = Vec::new();
        if let Err(error) = core.stop() {
            recovery_errors.push(error);
        }
        if let Err(error) = install.rollback() {
            recovery_errors.push(error);
        }
        if let Err(error) = core.start().and_then(|()| core.health()) {
            recovery_errors.push(error);
        }
        return Err(CommandFailure {
            cause,
            recovery: recovery_status(recovery_errors),
        });
    }

    install.commit().map_err(no_recovery)
}

pub fn update_geo_artifact<C: CoreControl>(
    core: &mut C,
    candidate: &Path,
    active: &Path,
    expected_sha256: &str,
    active_config: &Path,
) -> Result<(), CommandFailure> {
    let install =
        install_verified_artifact(candidate, active, expected_sha256).map_err(no_recovery)?;
    if let Err(cause) = core
        .apply_config(active_config)
        .and_then(|()| core.health())
    {
        let mut recovery_errors = Vec::new();
        if let Err(error) = install.rollback() {
            recovery_errors.push(error);
        }
        if let Err(error) = core
            .apply_config(active_config)
            .and_then(|()| core.health())
        {
            recovery_errors.push(error);
        }
        return Err(CommandFailure {
            cause,
            recovery: recovery_status(recovery_errors),
        });
    }
    install.commit().map_err(no_recovery)
}

fn restart_core<C: CoreControl>(core: &mut C) -> RecoveryStatus {
    core.start()
        .and_then(|()| core.health())
        .map(|()| RecoveryStatus::Restored)
        .unwrap_or_else(|error| RecoveryStatus::Failed { error })
}

fn recovery_status(mut errors: Vec<AppError>) -> RecoveryStatus {
    if errors.is_empty() {
        RecoveryStatus::Restored
    } else {
        RecoveryStatus::Failed {
            error: errors.remove(0),
        }
    }
}

pub struct SupervisorControl<'a> {
    supervisor: &'a mut CoreSupervisor,
    health_timeout: Duration,
}

impl<'a> SupervisorControl<'a> {
    pub fn new(supervisor: &'a mut CoreSupervisor, health_timeout: Duration) -> Self {
        Self {
            supervisor,
            health_timeout,
        }
    }
}

impl CoreControl for SupervisorControl<'_> {
    fn validate_candidate(&mut self, path: &Path) -> Result<(), AppError> {
        self.supervisor.validate_candidate(path)
    }

    fn apply_config(&mut self, path: &Path) -> Result<(), AppError> {
        self.supervisor.apply_config(path)
    }

    fn start(&mut self) -> Result<(), AppError> {
        self.supervisor.start()
    }

    fn health(&mut self) -> Result<(), AppError> {
        let deadline = Instant::now() + self.health_timeout;
        loop {
            match self.supervisor.health(Duration::from_millis(250)) {
                Ok(_) => return Ok(()),
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    fn stop(&mut self) -> Result<(), AppError> {
        self.supervisor.stop()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleFailure {
    pub cause: AppError,
    pub cleanup_errors: Vec<AppError>,
}

impl fmt::Display for LifecycleFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.cause)
    }
}

impl std::error::Error for LifecycleFailure {}

pub struct ApplicationLifecycle<'a, C, R, P> {
    core: &'a mut C,
    runtime: &'a mut R,
    system_proxy: &'a mut P,
}

impl<'a, C, R, P> ApplicationLifecycle<'a, C, R, P>
where
    C: CoreControl,
    R: RuntimeControl,
    P: SystemProxyControl,
{
    pub fn new(core: &'a mut C, runtime: &'a mut R, system_proxy: &'a mut P) -> Self {
        Self {
            core,
            runtime,
            system_proxy,
        }
    }

    pub fn start(&mut self) -> Result<(), LifecycleFailure> {
        if self.system_proxy.recovery_pending() {
            self.system_proxy
                .recover_pending()
                .map_err(lifecycle_failure)?;
        }
        self.core.start().map_err(lifecycle_failure)?;
        if let Err(cause) = self.core.health() {
            let cleanup_errors = self.core.stop().err().into_iter().collect();
            return Err(LifecycleFailure {
                cause,
                cleanup_errors,
            });
        }
        let topics = [
            RealtimeTopic::Traffic,
            RealtimeTopic::Memory,
            RealtimeTopic::Connections,
            RealtimeTopic::Logs,
        ];
        if let Err(cause) = self.runtime.start_realtime(&topics) {
            self.runtime.stop_realtime();
            let cleanup_errors = self.core.stop().err().into_iter().collect();
            return Err(LifecycleFailure {
                cause,
                cleanup_errors,
            });
        }
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<(), LifecycleFailure> {
        self.runtime.stop_realtime();
        let mut errors = Vec::new();
        if self.system_proxy.recovery_pending()
            && let Err(error) = self.system_proxy.recover_pending()
        {
            errors.push(error);
        }
        if let Err(error) = self.core.stop() {
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(LifecycleFailure {
                cause: errors.remove(0),
                cleanup_errors: errors,
            })
        }
    }
}

fn lifecycle_failure(cause: AppError) -> LifecycleFailure {
    LifecycleFailure {
        cause,
        cleanup_errors: Vec::new(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryStatus {
    NotNeeded,
    Restored,
    Failed { error: AppError },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyProfileResult {
    pub selected: ProfileId,
    pub recovery: RecoveryStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandFailure {
    pub cause: AppError,
    pub recovery: RecoveryStatus,
}

impl fmt::Display for CommandFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.cause)
    }
}

impl std::error::Error for CommandFailure {}

pub struct ConfigCommandHandler<'a, C> {
    profiles: &'a mut FileProfileStore,
    core: &'a mut C,
    runtime_credentials: Option<RuntimeCredentials>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeCredentials {
    pub controller: SocketAddr,
    pub secret: String,
}

impl<'a, C: CoreControl> ConfigCommandHandler<'a, C> {
    pub fn new(profiles: &'a mut FileProfileStore, core: &'a mut C) -> Self {
        Self {
            profiles,
            core,
            runtime_credentials: None,
        }
    }

    pub fn with_runtime_credentials(
        profiles: &'a mut FileProfileStore,
        core: &'a mut C,
        runtime_credentials: RuntimeCredentials,
    ) -> Self {
        Self {
            profiles,
            core,
            runtime_credentials: Some(runtime_credentials),
        }
    }

    pub fn apply_profile_yaml(
        &mut self,
        id: &ProfileId,
        yaml: &str,
        now: i64,
    ) -> Result<ApplyProfileResult, CommandFailure> {
        let previous_selected = self.profiles.selected().cloned();
        let candidate = self
            .profiles
            .prepare_candidate(id, yaml)
            .map_err(no_recovery)?;
        let runtime_candidate = self
            .runtime_credentials
            .as_ref()
            .map(|credentials| {
                self.profiles.prepare_runtime_candidate(
                    id,
                    yaml,
                    credentials.controller,
                    &credentials.secret,
                )
            })
            .transpose()
            .map_err(no_recovery)?;
        self.core
            .validate_candidate(
                runtime_candidate
                    .as_ref()
                    .map_or_else(|| candidate.path(), |candidate| candidate.path()),
            )
            .map_err(no_recovery)?;
        self.profiles
            .commit_candidate(candidate, now)
            .map_err(no_recovery)?;

        let active_path = self.active_path(id).map_err(no_recovery)?;
        if let Err(cause) = self.core.apply_config(&active_path) {
            return Err(self.recover(cause, id, previous_selected.as_ref(), now));
        }
        if let Err(cause) = self.core.health() {
            return Err(self.recover(cause, id, previous_selected.as_ref(), now));
        }
        if let Err(cause) = self.profiles.select(id) {
            return Err(self.recover(cause, id, previous_selected.as_ref(), now));
        }

        Ok(ApplyProfileResult {
            selected: id.clone(),
            recovery: RecoveryStatus::NotNeeded,
        })
    }

    pub fn select_profile(
        &mut self,
        id: &ProfileId,
        _now: i64,
    ) -> Result<ApplyProfileResult, CommandFailure> {
        let previous_selected = self.profiles.selected().cloned();
        let active_path = self.active_path(id).map_err(no_recovery)?;
        self.core
            .validate_candidate(&active_path)
            .map_err(no_recovery)?;
        if let Err(cause) = self.core.apply_config(&active_path) {
            return Err(self.recover_runtime(cause, previous_selected.as_ref()));
        }
        if let Err(cause) = self.core.health() {
            return Err(self.recover_runtime(cause, previous_selected.as_ref()));
        }
        if let Err(cause) = self.profiles.select(id) {
            return Err(self.recover_runtime(cause, previous_selected.as_ref()));
        }
        Ok(ApplyProfileResult {
            selected: id.clone(),
            recovery: RecoveryStatus::NotNeeded,
        })
    }

    pub fn update_profile_yaml(
        &mut self,
        id: &ProfileId,
        yaml: &str,
        now: i64,
    ) -> Result<(), CommandFailure> {
        if self.profiles.selected() == Some(id) {
            self.apply_profile_yaml(id, yaml, now)?;
            return Ok(());
        }
        let candidate = self
            .profiles
            .prepare_candidate(id, yaml)
            .map_err(no_recovery)?;
        let runtime_candidate = self
            .runtime_credentials
            .as_ref()
            .map(|credentials| {
                self.profiles.prepare_runtime_candidate(
                    id,
                    yaml,
                    credentials.controller,
                    &credentials.secret,
                )
            })
            .transpose()
            .map_err(no_recovery)?;
        self.core
            .validate_candidate(
                runtime_candidate
                    .as_ref()
                    .map_or_else(|| candidate.path(), |candidate| candidate.path()),
            )
            .map_err(no_recovery)?;
        self.profiles
            .commit_candidate(candidate, now)
            .map_err(no_recovery)
    }

    /// 更新全局 Merge 配置。存在激活配置时,与 UpdateProfileYaml 走相同的
    /// 候选校验/应用路径;校验或健康检查失败时恢复上一份 merge 配置。
    pub fn update_merge_config(&mut self, yaml: &str, _now: i64) -> Result<(), CommandFailure> {
        let previous_yaml = self.profiles.merge_yaml().map_err(no_recovery)?;
        self.profiles.set_merge_yaml(yaml).map_err(no_recovery)?;
        // 先对所有配置做纯函数合并,尽早暴露类型冲突,不依赖内核是否运行。
        let merged = self
            .profiles
            .list()
            .iter()
            .map(|profile| self.profiles.merged_yaml(&profile.id).map(|_| ()))
            .collect::<Result<Vec<_>, _>>();
        if let Err(cause) = merged {
            return Err(self.restore_merge(cause, &previous_yaml));
        }
        let Some(selected) = self.profiles.selected().cloned() else {
            return Ok(());
        };
        let source = self.profiles.yaml(&selected).map_err(no_recovery)?;
        let candidate = match &self.runtime_credentials {
            Some(credentials) => self.profiles.prepare_runtime_candidate(
                &selected,
                &source,
                credentials.controller,
                &credentials.secret,
            ),
            None => self.profiles.prepare_merge_candidate(&selected),
        }
        .map_err(no_recovery)?;
        if let Err(cause) = self.core.validate_candidate(candidate.path()) {
            return Err(self.restore_merge(cause, &previous_yaml));
        }
        let active_path = self.active_path(&selected).map_err(no_recovery)?;
        if let Err(cause) = self.core.apply_config(&active_path) {
            return Err(self.recover_merge(cause, &selected, &previous_yaml));
        }
        if let Err(cause) = self.core.health() {
            return Err(self.recover_merge(cause, &selected, &previous_yaml));
        }
        Ok(())
    }

    /// 内核尚未应用新配置时的回退:只恢复上一份 merge 配置。
    fn restore_merge(&mut self, cause: AppError, previous_yaml: &str) -> CommandFailure {
        let recovery = self
            .profiles
            .set_merge_yaml(previous_yaml)
            .map(|()| RecoveryStatus::Restored)
            .unwrap_or_else(|error| RecoveryStatus::Failed { error });
        CommandFailure { cause, recovery }
    }

    /// 新配置已应用但健康检查失败时的回退:恢复 merge 配置并重新应用旧运行配置。
    fn recover_merge(
        &mut self,
        cause: AppError,
        selected: &ProfileId,
        previous_yaml: &str,
    ) -> CommandFailure {
        let recovery = self
            .profiles
            .set_merge_yaml(previous_yaml)
            .and_then(|()| {
                let path = self.active_path(selected)?;
                self.core.apply_config(&path)?;
                self.core.health()
            })
            .map(|()| RecoveryStatus::Restored)
            .unwrap_or_else(|error| RecoveryStatus::Failed { error });
        CommandFailure { cause, recovery }
    }

    fn active_path(&self, id: &ProfileId) -> Result<std::path::PathBuf, AppError> {
        match &self.runtime_credentials {
            Some(credentials) => {
                self.profiles
                    .materialize_runtime(id, credentials.controller, &credentials.secret)
            }
            None => self.profiles.yaml_path(id),
        }
    }

    fn recover_runtime(
        &mut self,
        cause: AppError,
        previous_selected: Option<&ProfileId>,
    ) -> CommandFailure {
        let recovery = match previous_selected {
            Some(previous) => self
                .active_path(previous)
                .and_then(|path| self.core.apply_config(&path))
                .and_then(|()| self.core.health())
                .map(|()| RecoveryStatus::Restored)
                .unwrap_or_else(|error| RecoveryStatus::Failed { error }),
            None => self
                .core
                .stop()
                .map(|()| RecoveryStatus::Restored)
                .unwrap_or_else(|error| RecoveryStatus::Failed { error }),
        };
        CommandFailure { cause, recovery }
    }

    fn recover(
        &mut self,
        cause: AppError,
        changed: &ProfileId,
        previous_selected: Option<&ProfileId>,
        now: i64,
    ) -> CommandFailure {
        let recovery = self
            .restore_file_and_runtime(changed, previous_selected, now)
            .map(|()| RecoveryStatus::Restored)
            .unwrap_or_else(|error| RecoveryStatus::Failed { error });
        CommandFailure { cause, recovery }
    }

    fn restore_file_and_runtime(
        &mut self,
        changed: &ProfileId,
        previous_selected: Option<&ProfileId>,
        now: i64,
    ) -> Result<(), AppError> {
        self.profiles.rollback_to_last_good(changed, now)?;
        match previous_selected {
            Some(previous) => {
                let path = self.active_path(previous)?;
                self.core.apply_config(&path)?;
                self.core.health()?;
                self.profiles.select(previous)
            }
            None => self.core.stop(),
        }
    }
}

pub struct ProfileCommandHandler<'a, C> {
    profiles: &'a mut FileProfileStore,
    core: &'a mut C,
    runtime_credentials: Option<RuntimeCredentials>,
}

pub trait ProfileFetcher {
    /// 拉取订阅内容。`user_agent` 覆盖客户端默认 UA（None 用默认值）。
    fn fetch(&mut self, url: &str, user_agent: Option<&str>) -> Result<String, AppError>;
}

pub struct ReqwestProfileFetcher {
    client: reqwest::blocking::Client,
    max_bytes: u64,
}

impl ReqwestProfileFetcher {
    pub fn new(timeout: Duration, max_bytes: u64) -> Result<Self, AppError> {
        if timeout.is_zero() || max_bytes == 0 {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "profile fetch timeout and size limit must be greater than zero",
            ));
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .user_agent(concat!("clash-verge/v", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(profile_fetch_error)?;
        Ok(Self { client, max_bytes })
    }
}

impl ProfileFetcher for ReqwestProfileFetcher {
    fn fetch(&mut self, url: &str, user_agent: Option<&str>) -> Result<String, AppError> {
        let parsed = reqwest::Url::parse(url).map_err(profile_fetch_error)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "profile URL must use HTTP or HTTPS",
            ));
        }
        let mut request = self.client.get(parsed);
        if let Some(ua) = user_agent.filter(|ua| !ua.trim().is_empty()) {
            request = request.header(reqwest::header::USER_AGENT, ua);
        }
        let mut response = request
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(profile_fetch_error)?;
        if response
            .content_length()
            .is_some_and(|length| length > self.max_bytes)
        {
            return Err(AppError::new(
                ErrorCode::ValidationFailed,
                "remote profile exceeds the configured size limit",
            ));
        }
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(self.max_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(profile_fetch_error)?;
        if bytes.len() as u64 > self.max_bytes {
            return Err(AppError::new(
                ErrorCode::ValidationFailed,
                "remote profile exceeds the configured size limit",
            ));
        }
        String::from_utf8(bytes).map_err(profile_fetch_error)
    }
}

fn profile_fetch_error(error: impl fmt::Display) -> AppError {
    AppError::new(ErrorCode::CoreUnavailable, error.to_string())
}

pub trait ArtifactFetcher {
    fn fetch(&mut self, url: &str) -> Result<Vec<u8>, AppError>;
}

pub struct ReqwestArtifactFetcher {
    client: reqwest::blocking::Client,
    max_bytes: u64,
    allowed_hosts: BTreeSet<String>,
}

impl ReqwestArtifactFetcher {
    pub fn new(
        timeout: Duration,
        max_bytes: u64,
        allowed_hosts: impl IntoIterator<Item = String>,
    ) -> Result<Self, AppError> {
        let allowed_hosts = allowed_hosts.into_iter().collect::<BTreeSet<_>>();
        if timeout.is_zero() || max_bytes == 0 || allowed_hosts.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "artifact fetch timeout, size limit, and host allowlist are required",
            ));
        }
        if allowed_hosts
            .iter()
            .any(|host| host.is_empty() || host.contains('/') || host.contains(':'))
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "artifact host allowlist contains an invalid host",
            ));
        }
        let redirect_hosts = allowed_hosts.clone();
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= 5 {
                    return attempt.error("too many artifact redirects");
                }
                let url = attempt.url();
                if url.scheme() == "https"
                    && url
                        .host_str()
                        .is_some_and(|host| redirect_hosts.contains(host))
                    && url.username().is_empty()
                    && url.password().is_none()
                {
                    attempt.follow()
                } else {
                    attempt.error("artifact redirect left the HTTPS host allowlist")
                }
            }))
            .user_agent("Verge/0.1 artifact-updater")
            .build()
            .map_err(profile_fetch_error)?;
        Ok(Self {
            client,
            max_bytes,
            allowed_hosts,
        })
    }

    fn validate_url(&self, url: &str) -> Result<reqwest::Url, AppError> {
        let parsed = reqwest::Url::parse(url).map_err(profile_fetch_error)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "artifact URL has no host"))?;
        if parsed.scheme() != "https" || !self.allowed_hosts.contains(host) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "artifact URL must use HTTPS and an allowlisted host",
            ));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "artifact URL must not contain credentials",
            ));
        }
        Ok(parsed)
    }
}

impl ArtifactFetcher for ReqwestArtifactFetcher {
    fn fetch(&mut self, url: &str) -> Result<Vec<u8>, AppError> {
        let parsed = self.validate_url(url)?;
        let mut response = self
            .client
            .get(parsed)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(profile_fetch_error)?;
        if response
            .content_length()
            .is_some_and(|length| length > self.max_bytes)
        {
            return Err(AppError::new(
                ErrorCode::ValidationFailed,
                "remote artifact exceeds the configured size limit",
            ));
        }
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(self.max_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(profile_fetch_error)?;
        if bytes.len() as u64 > self.max_bytes {
            return Err(AppError::new(
                ErrorCode::ValidationFailed,
                "remote artifact exceeds the configured size limit",
            ));
        }
        Ok(bytes)
    }
}

pub fn download_mihomo_candidate<F: ArtifactFetcher>(
    fetcher: &mut F,
    target: &SidecarTarget,
    staging_directory: &Path,
) -> Result<PathBuf, AppError> {
    fs::create_dir_all(staging_directory).map_err(profile_fetch_error)?;
    let archive_path = staging_directory.join("mihomo-download.gz");
    let candidate_path = staging_directory.join("mihomo-candidate");
    let archive = fetcher.fetch(&target.download_url)?;
    fs::write(&archive_path, &archive).map_err(profile_fetch_error)?;
    target.verify_archive(&archive_path)?;

    let mut decoder = flate2::read::GzDecoder::new(Cursor::new(archive));
    let mut executable = Vec::new();
    decoder
        .by_ref()
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut executable)
        .map_err(profile_fetch_error)?;
    if executable.len() > 128 * 1024 * 1024 {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            "decompressed Mihomo executable exceeds the size limit",
        ));
    }
    fs::write(&candidate_path, executable).map_err(profile_fetch_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&candidate_path, fs::Permissions::from_mode(0o755))
            .map_err(profile_fetch_error)?;
    }
    target.verify_executable(&candidate_path)?;
    let _ = fs::remove_file(archive_path);
    Ok(candidate_path)
}

pub struct ProfileUpdateCoordinator<'a, C, F> {
    profiles: &'a mut FileProfileStore,
    core: &'a mut C,
    scheduler: &'a mut UpdateScheduler,
    fetcher: &'a mut F,
    runtime_credentials: Option<RuntimeCredentials>,
}

pub type ProfileUpdateOutcome = (ProfileId, Result<(), CommandFailure>);

impl<'a, C: CoreControl, F: ProfileFetcher> ProfileUpdateCoordinator<'a, C, F> {
    pub fn new(
        profiles: &'a mut FileProfileStore,
        core: &'a mut C,
        scheduler: &'a mut UpdateScheduler,
        fetcher: &'a mut F,
        runtime_credentials: Option<RuntimeCredentials>,
    ) -> Self {
        Self {
            profiles,
            core,
            scheduler,
            fetcher,
            runtime_credentials,
        }
    }

    pub fn update_manual(&mut self, id: &ProfileId, now: i64) -> Result<(), CommandFailure> {
        let job = self
            .scheduler
            .claim_manual(self.profiles, id)
            .map_err(no_recovery)?;
        self.run_job(job, now)
    }

    pub fn update_due(
        &mut self,
        now: i64,
        trigger: UpdateTrigger,
    ) -> Result<Vec<ProfileUpdateOutcome>, AppError> {
        let jobs = self.scheduler.claim_due(self.profiles, now, trigger)?;
        Ok(jobs
            .into_iter()
            .map(|job| {
                let id = job.id.clone();
                (id, self.run_job(job, now))
            })
            .collect())
    }

    fn run_job(&mut self, job: ProfileUpdateJob, now: i64) -> Result<(), CommandFailure> {
        let fetched = self.fetcher.fetch(&job.url, job.user_agent.as_deref());
        let result = match fetched {
            Ok(yaml) => self.config().update_profile_yaml(&job.id, &yaml, now),
            Err(cause) => Err(no_recovery(cause)),
        };
        let persistence_error = result.as_ref().err().and_then(|failure| {
            self.profiles
                .mark_update_failed(&job.id, now, &failure.cause)
                .err()
                .map(|error| CommandFailure {
                    cause: failure.cause.clone(),
                    recovery: RecoveryStatus::Failed { error },
                })
        });
        self.scheduler.finish(&job.id);
        if let Some(error) = persistence_error {
            return Err(error);
        }
        result
    }

    fn config(&mut self) -> ConfigCommandHandler<'_, C> {
        match self.runtime_credentials.clone() {
            Some(credentials) => ConfigCommandHandler::with_runtime_credentials(
                self.profiles,
                self.core,
                credentials,
            ),
            None => ConfigCommandHandler::new(self.profiles, self.core),
        }
    }
}

impl<'a, C: CoreControl> ProfileCommandHandler<'a, C> {
    pub fn new(
        profiles: &'a mut FileProfileStore,
        core: &'a mut C,
        runtime_credentials: Option<RuntimeCredentials>,
    ) -> Self {
        Self {
            profiles,
            core,
            runtime_credentials,
        }
    }

    pub fn execute(
        &mut self,
        command: AppCommand,
        now: i64,
    ) -> Result<AppCommandResult, CommandFailure> {
        let output = match command {
            AppCommand::GetRuntimeSettings
            | AppCommand::GetApplicationSettings
            | AppCommand::GetHelperStatus
            | AppCommand::UpdateApplicationSettings { .. }
            | AppCommand::ExportApplicationSettings { .. }
            | AppCommand::PreviewApplicationSettingsImport { .. }
            | AppCommand::ImportApplicationSettings { .. }
            | AppCommand::ResetApplicationSettingsScope { .. }
            | AppCommand::ExportDiagnostics { .. }
            | AppCommand::ExportEncryptedBackup { .. }
            | AppCommand::RestoreEncryptedBackup { .. }
            | AppCommand::UpdateMihomo => {
                return Err(no_recovery(AppError::new(
                    ErrorCode::InvalidInput,
                    "application settings are owned by the application backend",
                )));
            }
            AppCommand::ListProfiles => AppCommandOutput::Profiles {
                profiles: self.profiles.list().to_vec(),
                selected: self.profiles.selected().cloned(),
            },
            AppCommand::GetProfileYaml { id } => AppCommandOutput::ProfileYaml {
                yaml: self.profiles.yaml(&id).map_err(no_recovery)?,
                id,
            },
            AppCommand::ImportProfile {
                id,
                name,
                yaml,
                source,
                update_policy,
            } => {
                let profile =
                    Profile::new(id, name, source, update_policy, now, None).map_err(no_recovery)?;
                self.profiles.import(profile, &yaml).map_err(no_recovery)?;
                AppCommandOutput::None
            }
            AppCommand::ImportRemoteProfile { .. } | AppCommand::UpdateRemoteProfile { .. } => {
                return Err(no_recovery(AppError::new(
                    ErrorCode::InvalidInput,
                    "remote profile commands require a profile fetcher",
                )));
            }
            AppCommand::SetProfileUpdatePolicy { id, update_policy } => {
                self.profiles
                    .set_update_policy(&id, update_policy, now)
                    .map_err(no_recovery)?;
                AppCommandOutput::None
            }
            AppCommand::SelectProfile { id } => {
                self.config().select_profile(&id, now)?;
                AppCommandOutput::None
            }
            AppCommand::UpdateProfileYaml { id, yaml } => {
                self.config().update_profile_yaml(&id, &yaml, now)?;
                AppCommandOutput::None
            }
            AppCommand::GetMergeConfig => AppCommandOutput::MergeConfigYaml {
                yaml: self.profiles.merge_yaml().map_err(no_recovery)?,
            },
            AppCommand::GetMergedProfileYaml { id } => AppCommandOutput::MergedProfileYaml {
                yaml: self.profiles.merged_yaml(&id).map_err(no_recovery)?,
                id,
            },
            AppCommand::UpdateMergeConfig { yaml } => {
                self.config().update_merge_config(&yaml, now)?;
                AppCommandOutput::None
            }
            AppCommand::DeleteProfile { id } => {
                if self.profiles.selected() == Some(&id) {
                    return Err(no_recovery(AppError::new(
                        ErrorCode::Conflict,
                        "select another profile before deleting the active profile",
                    )));
                }
                self.profiles.delete(&id).map_err(no_recovery)?;
                AppCommandOutput::None
            }
        };
        Ok(AppCommandResult {
            output,
            summary: "Profile command completed".into(),
        })
    }

    fn config(&mut self) -> ConfigCommandHandler<'_, C> {
        match self.runtime_credentials.clone() {
            Some(credentials) => ConfigCommandHandler::with_runtime_credentials(
                self.profiles,
                self.core,
                credentials,
            ),
            None => ConfigCommandHandler::new(self.profiles, self.core),
        }
    }
}

fn no_recovery(cause: AppError) -> CommandFailure {
    CommandFailure {
        cause,
        recovery: RecoveryStatus::NotNeeded,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        fs,
        io::Write as _,
        net::TcpListener,
        path::PathBuf,
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    use verge_config::{Profile, ProfileSource, UpdatePolicy};
    use verge_domain::ErrorCode;

    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "verge-application-{name}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Default)]
    struct FakeCore {
        start: VecDeque<Result<(), AppError>>,
        stop: VecDeque<Result<(), AppError>>,
        validation: VecDeque<Result<(), AppError>>,
        apply: VecDeque<Result<(), AppError>>,
        health: VecDeque<Result<(), AppError>>,
        applied_yaml: Vec<String>,
        validated_yaml: Vec<String>,
        stop_calls: usize,
        start_calls: usize,
    }

    impl CoreControl for FakeCore {
        fn validate_candidate(&mut self, path: &Path) -> Result<(), AppError> {
            self.validation.pop_front().unwrap_or(Ok(()))?;
            self.validated_yaml.push(fs::read_to_string(path).unwrap());
            Ok(())
        }

        fn apply_config(&mut self, path: &Path) -> Result<(), AppError> {
            self.applied_yaml.push(fs::read_to_string(path).unwrap());
            self.apply.pop_front().unwrap_or(Ok(()))
        }

        fn start(&mut self) -> Result<(), AppError> {
            self.start_calls += 1;
            self.start.pop_front().unwrap_or(Ok(()))
        }

        fn health(&mut self) -> Result<(), AppError> {
            self.health.pop_front().unwrap_or(Ok(()))
        }

        fn stop(&mut self) -> Result<(), AppError> {
            self.stop_calls += 1;
            self.stop.pop_front().unwrap_or(Ok(()))
        }
    }

    fn failure(message: &str) -> AppError {
        AppError::new(ErrorCode::CoreUnavailable, message)
    }

    fn sha256(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes))
    }

    #[test]
    fn mihomo_artifact_update_commits_only_after_health_check() {
        let directory = TestDir::new("mihomo-artifact-success");
        let candidate = directory.0.join("candidate");
        let active = directory.0.join("mihomo");
        fs::write(&candidate, b"new binary").unwrap();
        fs::write(&active, b"old binary").unwrap();
        let mut core = FakeCore::default();

        update_mihomo_artifact(&mut core, &candidate, &active, &sha256(b"new binary")).unwrap();

        assert_eq!(fs::read(&active).unwrap(), b"new binary");
        assert!(!active.with_extension("previous").exists());
        assert_eq!(core.stop_calls, 1);
        assert_eq!(core.start_calls, 1);
    }

    #[test]
    fn unhealthy_mihomo_artifact_restores_and_restarts_previous_binary() {
        let directory = TestDir::new("mihomo-artifact-rollback");
        let candidate = directory.0.join("candidate");
        let active = directory.0.join("mihomo");
        fs::write(&candidate, b"bad binary").unwrap();
        fs::write(&active, b"old binary").unwrap();
        let mut core = FakeCore::default();
        core.health.push_back(Err(failure("new binary unhealthy")));
        core.health.push_back(Ok(()));

        let error = update_mihomo_artifact(&mut core, &candidate, &active, &sha256(b"bad binary"))
            .unwrap_err();

        assert_eq!(error.cause.message, "new binary unhealthy");
        assert_eq!(error.recovery, RecoveryStatus::Restored);
        assert_eq!(fs::read(&active).unwrap(), b"old binary");
        assert_eq!(core.stop_calls, 2);
        assert_eq!(core.start_calls, 2);
    }

    #[test]
    fn rejected_mihomo_digest_restarts_untouched_binary() {
        let directory = TestDir::new("mihomo-artifact-digest");
        let candidate = directory.0.join("candidate");
        let active = directory.0.join("mihomo");
        fs::write(&candidate, b"candidate").unwrap();
        fs::write(&active, b"old binary").unwrap();
        let mut core = FakeCore::default();

        let error =
            update_mihomo_artifact(&mut core, &candidate, &active, &"0".repeat(64)).unwrap_err();

        assert_eq!(error.cause.code, ErrorCode::ValidationFailed);
        assert_eq!(error.recovery, RecoveryStatus::Restored);
        assert_eq!(fs::read(&active).unwrap(), b"old binary");
        assert_eq!(core.start_calls, 1);
    }

    #[test]
    fn geo_artifact_reload_failure_restores_previous_data() {
        let directory = TestDir::new("geo-artifact-rollback");
        let candidate = directory.0.join("candidate.mmdb");
        let active = directory.0.join("Country.mmdb");
        let config = directory.0.join("config.yaml");
        fs::write(&candidate, b"bad geo").unwrap();
        fs::write(&active, b"old geo").unwrap();
        fs::write(&config, b"mode: rule\n").unwrap();
        let mut core = FakeCore::default();
        core.apply.push_back(Err(failure("reload failed")));
        core.apply.push_back(Ok(()));

        let error =
            update_geo_artifact(&mut core, &candidate, &active, &sha256(b"bad geo"), &config)
                .unwrap_err();

        assert_eq!(error.cause.message, "reload failed");
        assert_eq!(error.recovery, RecoveryStatus::Restored);
        assert_eq!(fs::read(&active).unwrap(), b"old geo");
        assert_eq!(core.applied_yaml.len(), 2);
    }

    fn store_with_profiles(directory: &Path) -> (FileProfileStore, ProfileId, ProfileId) {
        let mut store = FileProfileStore::open(directory).unwrap();
        let first = ProfileId::parse("first").unwrap();
        let second = ProfileId::parse("second").unwrap();
        for (id, name, yaml) in [
            (&first, "First", "mode: rule\n"),
            (&second, "Second", "mode: direct\n"),
        ] {
            let profile = Profile::new(
                id.clone(),
                name,
                ProfileSource::Local,
                UpdatePolicy::Manual,
                1_000,
                    None)
            .unwrap();
            store.import(profile, yaml).unwrap();
        }
        store.select(&first).unwrap();
        (store, first, second)
    }

    #[test]
    fn healthy_candidate_is_committed_and_selected() {
        let directory = TestDir::new("success");
        let (mut store, _, second) = store_with_profiles(&directory.0);
        let mut core = FakeCore::default();
        let mut handler = ConfigCommandHandler::new(&mut store, &mut core);

        let result = handler
            .apply_profile_yaml(&second, "mode: global\n", 2_000)
            .unwrap();

        assert_eq!(result.selected, second);
        assert_eq!(store.selected(), Some(&second));
        assert_eq!(store.yaml(&second).unwrap(), "mode: global\n");
    }

    #[test]
    fn profile_commands_list_import_and_read_yaml() {
        let directory = TestDir::new("profile-commands");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let mut core = FakeCore::default();
        let id = ProfileId::parse("local").unwrap();
        let mut handler = ProfileCommandHandler::new(&mut store, &mut core, None);

        handler
            .execute(
                AppCommand::ImportProfile {
                    id: id.clone(),
                    name: "Local".into(),
                    yaml: "mode: rule\n".into(),
                    source: verge_domain::ProfileSource::Local,
                    update_policy: verge_domain::UpdatePolicy::Manual,
                },
                100,
            )
            .unwrap();
        let listed = handler.execute(AppCommand::ListProfiles, 100).unwrap();
        assert!(matches!(
            listed.output,
            AppCommandOutput::Profiles { profiles, selected: None } if profiles.len() == 1
        ));
        let yaml = handler
            .execute(AppCommand::GetProfileYaml { id: id.clone() }, 100)
            .unwrap();
        assert!(matches!(
            yaml.output,
            AppCommandOutput::ProfileYaml { id: output_id, yaml }
                if output_id == id && yaml == "mode: rule\n"
        ));
    }

    #[test]
    fn application_owned_commands_are_rejected_by_profile_handler() {
        let directory = TestDir::new("app-owned-commands");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let mut core = FakeCore::default();
        let mut handler = ProfileCommandHandler::new(&mut store, &mut core, None);
        for command in [
            AppCommand::ExportApplicationSettings {
                destination: "/tmp/settings.json".into(),
            },
            AppCommand::PreviewApplicationSettingsImport {
                source: "/tmp/settings.json".into(),
            },
            AppCommand::ImportApplicationSettings {
                source: "/tmp/settings.json".into(),
            },
            AppCommand::ResetApplicationSettingsScope {
                scope: verge_domain::SettingsScope::Appearance,
            },
        ] {
            let error = handler.execute(command, 100).unwrap_err();
            assert_eq!(error.cause.code, ErrorCode::InvalidInput);
        }
    }

    #[test]
    fn profile_select_and_update_apply_injected_runtime_config() {
        let directory = TestDir::new("profile-runtime");
        let (mut store, _, second) = store_with_profiles(&directory.0);
        let mut core = FakeCore::default();
        let credentials = RuntimeCredentials {
            controller: "127.0.0.1:45678".parse().unwrap(),
            secret: "runtime-secret".into(),
        };
        let mut handler = ProfileCommandHandler::new(&mut store, &mut core, Some(credentials));

        handler
            .execute(AppCommand::SelectProfile { id: second.clone() }, 200)
            .unwrap();
        handler
            .execute(
                AppCommand::UpdateProfileYaml {
                    id: second.clone(),
                    yaml: "mode: global\n".into(),
                },
                201,
            )
            .unwrap();

        assert_eq!(store.selected(), Some(&second));
        assert_eq!(store.yaml(&second).unwrap(), "mode: global\n");
        assert!(core.applied_yaml.iter().all(|yaml| {
            yaml.contains("external-controller: 127.0.0.1:45678")
                && yaml.contains("secret: runtime-secret")
        }));
        assert!(core.validated_yaml.iter().all(|yaml| {
            yaml.contains("external-controller: 127.0.0.1:45678")
                && yaml.contains("secret: runtime-secret")
        }));
    }

    #[test]
    fn updating_inactive_profile_validates_without_switching_runtime() {
        let directory = TestDir::new("inactive-update");
        let (mut store, first, second) = store_with_profiles(&directory.0);
        let mut core = FakeCore::default();
        let mut handler = ProfileCommandHandler::new(
            &mut store,
            &mut core,
            Some(RuntimeCredentials {
                controller: "127.0.0.1:45678".parse().unwrap(),
                secret: "runtime-secret".into(),
            }),
        );

        handler
            .execute(
                AppCommand::UpdateProfileYaml {
                    id: second.clone(),
                    yaml: "mode: global\n".into(),
                },
                300,
            )
            .unwrap();

        assert_eq!(store.selected(), Some(&first));
        assert_eq!(store.yaml(&second).unwrap(), "mode: global\n");
        assert!(core.applied_yaml.is_empty());
        assert_eq!(core.validated_yaml.len(), 1);
        assert!(core.validated_yaml[0].contains("secret: runtime-secret"));
    }

    struct FakeFetcher {
        responses: VecDeque<Result<String, AppError>>,
    }

    impl ProfileFetcher for FakeFetcher {
        fn fetch(&mut self, _url: &str, _user_agent: Option<&str>) -> Result<String, AppError> {
            self.responses.pop_front().unwrap()
        }
    }

    #[test]
    fn remote_update_uses_same_transaction_and_records_retry_after_failure() {
        let directory = TestDir::new("remote-update");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let id = ProfileId::parse("remote").unwrap();
        store
            .import(
                Profile::new(
                    id.clone(),
                    "Remote",
                    ProfileSource::Remote {
                        url: "https://example.com/profile.yaml".into(),
                    },
                    UpdatePolicy::Interval { seconds: 300 },
                    100,
                    None)
                .unwrap(),
                "mode: rule\n",
            )
            .unwrap();
        let mut core = FakeCore::default();
        let mut scheduler = UpdateScheduler::default();
        let mut fetcher = FakeFetcher {
            responses: VecDeque::from([Err(failure("offline")), Ok("mode: global\n".into())]),
        };

        let first = ProfileUpdateCoordinator::new(
            &mut store,
            &mut core,
            &mut scheduler,
            &mut fetcher,
            None,
        )
        .update_manual(&id, 200)
        .unwrap_err();
        assert_eq!(first.cause.message, "offline");
        assert_eq!(store.list()[0].next_update_at, Some(260));

        ProfileUpdateCoordinator::new(&mut store, &mut core, &mut scheduler, &mut fetcher, None)
            .update_manual(&id, 201)
            .unwrap();
        assert_eq!(store.yaml(&id).unwrap(), "mode: global\n");
        assert_eq!(store.list()[0].consecutive_failures, 0);
        assert_eq!(store.list()[0].last_error, None);
    }

    fn serve_once(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        format!("http://{address}/profile.yaml")
    }

    #[test]
    fn http_profile_fetcher_enforces_protocol_and_size_limit() {
        let mut fetcher = ReqwestProfileFetcher::new(Duration::from_secs(2), 64).unwrap();
        assert_eq!(
            fetcher.fetch(&serve_once("mode: rule\n"), None).unwrap(),
            "mode: rule\n"
        );
        assert_eq!(
            fetcher.fetch("file:///tmp/profile.yaml", None).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        let mut limited = ReqwestProfileFetcher::new(Duration::from_secs(2), 4).unwrap();
        assert_eq!(
            limited.fetch(&serve_once("mode: rule\n"), None).unwrap_err().code,
            ErrorCode::ValidationFailed
        );
    }

    struct FakeArtifactFetcher {
        bytes: Vec<u8>,
    }

    impl ArtifactFetcher for FakeArtifactFetcher {
        fn fetch(&mut self, _url: &str) -> Result<Vec<u8>, AppError> {
            Ok(self.bytes.clone())
        }
    }

    #[test]
    fn artifact_fetcher_rejects_non_https_credentials_and_unknown_hosts() {
        let fetcher =
            ReqwestArtifactFetcher::new(Duration::from_secs(2), 1024, ["github.com".to_owned()])
                .unwrap();
        assert!(
            fetcher
                .validate_url("https://github.com/owner/release")
                .is_ok()
        );
        for url in [
            "http://github.com/owner/release",
            "https://example.com/artifact",
            "https://user:secret@github.com/artifact",
        ] {
            assert_eq!(
                fetcher.validate_url(url).unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }
    }

    #[test]
    fn mihomo_download_verifies_archive_and_decompressed_executable() {
        use flate2::{Compression, write::GzEncoder};

        let directory = TestDir::new("mihomo-download");
        let executable = b"mihomo executable";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(executable).unwrap();
        let archive = encoder.finish().unwrap();
        let target = SidecarTarget {
            asset: "mihomo.gz".into(),
            download_url: "https://github.com/MetaCubeX/mihomo/releases/download/v1/mihomo.gz"
                .into(),
            archive_sha256: sha256(&archive),
            executable_sha256: sha256(executable),
        };
        let mut fetcher = FakeArtifactFetcher { bytes: archive };

        let candidate = download_mihomo_candidate(&mut fetcher, &target, &directory.0).unwrap();

        assert_eq!(fs::read(candidate).unwrap(), executable);
    }

    #[test]
    fn mihomo_download_rejects_tampered_archive_before_extraction() {
        let directory = TestDir::new("mihomo-download-tampered");
        let target = SidecarTarget {
            asset: "mihomo.gz".into(),
            download_url: "https://github.com/MetaCubeX/mihomo/releases/download/v1/mihomo.gz"
                .into(),
            archive_sha256: "0".repeat(64),
            executable_sha256: "0".repeat(64),
        };
        let mut fetcher = FakeArtifactFetcher {
            bytes: b"not trusted".to_vec(),
        };

        assert!(download_mihomo_candidate(&mut fetcher, &target, &directory.0).is_err());
        assert!(!directory.0.join("mihomo-candidate").exists());
    }

    #[test]
    fn realtime_logs_redact_secrets_subscriptions_headers_and_home_paths() {
        let event = RealtimeEvent::Log(verge_domain::LogEvent {
            level: "info".into(),
            payload: "secret-value https://example.com/sub Authorization: Bearer token /Users/alice/config"
                .into(),
        });
        let redacted = redact_realtime_event(
            event,
            "secret-value",
            &["https://example.com/sub".into(), "/Users/alice".into()],
        );
        let RealtimeEvent::Log(log) = redacted else {
            panic!("expected log event");
        };
        assert!(!log.payload.contains("secret-value"));
        assert!(!log.payload.contains("example.com"));
        assert!(!log.payload.contains("token"));
        assert!(!log.payload.contains("/Users/alice"));
    }

    fn credentials() -> RuntimeCredentials {
        RuntimeCredentials {
            controller: "127.0.0.1:45678".parse().unwrap(),
            secret: "runtime-secret".into(),
        }
    }

    #[test]
    fn merge_update_validates_and_applies_merged_runtime_to_active_profile() {
        let directory = TestDir::new("merge-apply");
        let (mut store, first, _) = store_with_profiles(&directory.0);
        let mut core = FakeCore::default();
        let mut handler =
            ConfigCommandHandler::with_runtime_credentials(&mut store, &mut core, credentials());

        handler
            .update_merge_config(
                "rules:\n  - key: rules\n    op: prepend\n    items:\n      - DOMAIN,example.com,REJECT\n",
                2_000,
            )
            .unwrap();

        assert_eq!(store.merge().rules.len(), 1);
        assert_eq!(store.selected(), Some(&first));
        // 源配置不变,内核收到的是合并后的运行配置。
        assert_eq!(store.yaml(&first).unwrap(), "mode: rule\n");
        assert_eq!(core.validated_yaml.len(), 1);
        assert!(core.validated_yaml[0].contains("DOMAIN,example.com,REJECT"));
        assert!(core.validated_yaml[0].contains("secret: runtime-secret"));
        assert_eq!(core.applied_yaml.len(), 1);
        assert!(core.applied_yaml[0].contains("DOMAIN,example.com,REJECT"));
    }

    #[test]
    fn merge_update_without_selection_only_persists_config() {
        let directory = TestDir::new("merge-inactive");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        store
            .import(
                Profile::new(
                    ProfileId::parse("local").unwrap(),
                    "Local",
                    ProfileSource::Local,
                    UpdatePolicy::Manual,
                    100,
                    None)
                .unwrap(),
                "mode: rule\n",
            )
            .unwrap();
        let mut core = FakeCore::default();
        let mut handler = ConfigCommandHandler::new(&mut store, &mut core);

        handler
            .update_merge_config(
                "rules:\n  - key: mode\n    op: override\n    value: global\n",
                2_000,
            )
            .unwrap();

        assert_eq!(store.merge().rules.len(), 1);
        assert!(core.validated_yaml.is_empty());
        assert!(core.applied_yaml.is_empty());
    }

    #[test]
    fn unhealthy_merge_candidate_restores_previous_merge_and_runtime() {
        let directory = TestDir::new("merge-rollback");
        let (mut store, first, _) = store_with_profiles(&directory.0);
        let mut core = FakeCore::default();
        core.health.push_back(Err(failure("merged config unhealthy")));
        core.health.push_back(Ok(()));
        let mut handler =
            ConfigCommandHandler::with_runtime_credentials(&mut store, &mut core, credentials());

        let error = handler
            .update_merge_config(
                "rules:\n  - key: mode\n    op: override\n    value: global\n",
                2_000,
            )
            .unwrap_err();

        assert_eq!(error.cause.message, "merged config unhealthy");
        assert_eq!(error.recovery, RecoveryStatus::Restored);
        assert_eq!(store.merge(), &verge_config::MergeConfig::default());
        assert_eq!(store.yaml(&first).unwrap(), "mode: rule\n");
        assert_eq!(core.applied_yaml.len(), 2);
        assert!(core.applied_yaml[0].contains("mode: global"));
        assert!(core.applied_yaml[1].contains("mode: rule"));
    }

    #[test]
    fn conflicting_merge_is_rejected_and_restored_before_core_apply() {
        let directory = TestDir::new("merge-conflict");
        let (mut store, first, _) = store_with_profiles(&directory.0);
        let mut core = FakeCore::default();
        let mut handler = ConfigCommandHandler::new(&mut store, &mut core);

        let error = handler
            .update_merge_config(
                "rules:\n  - key: mode\n    op: prepend\n    items:\n      - x\n",
                2_000,
            )
            .unwrap_err();

        assert_eq!(error.cause.code, ErrorCode::ValidationFailed);
        assert_eq!(error.recovery, RecoveryStatus::Restored);
        assert_eq!(store.merge(), &verge_config::MergeConfig::default());
        assert_eq!(store.selected(), Some(&first));
        assert!(core.validated_yaml.is_empty());
        assert!(core.applied_yaml.is_empty());
    }

    #[test]
    fn profile_commands_route_merge_read_and_write() {
        let directory = TestDir::new("merge-commands");
        let (mut store, first, _) = store_with_profiles(&directory.0);
        let mut core = FakeCore::default();
        let mut handler = ProfileCommandHandler::new(&mut store, &mut core, Some(credentials()));

        let loaded = handler.execute(AppCommand::GetMergeConfig, 100).unwrap();
        assert!(matches!(
            loaded.output,
            AppCommandOutput::MergeConfigYaml { ref yaml } if yaml.contains("rules: []")
        ));

        handler
            .execute(
                AppCommand::UpdateMergeConfig {
                    yaml: "rules:\n  - key: mode\n    op: override\n    value: global\n".into(),
                },
                101,
            )
            .unwrap();

        let merged = handler
            .execute(
                AppCommand::GetMergedProfileYaml { id: first.clone() },
                102,
            )
            .unwrap();
        assert!(matches!(
            merged.output,
            AppCommandOutput::MergedProfileYaml { ref id, ref yaml }
                if *id == first && yaml.contains("mode: global") && !yaml.contains("runtime-secret")
        ));
        drop(handler);
        assert!(core.applied_yaml[0].contains("mode: global"));
    }

    #[test]
    fn rejected_candidate_never_replaces_profile() {
        let directory = TestDir::new("rejected");
        let (mut store, first, second) = store_with_profiles(&directory.0);
        let mut core = FakeCore::default();
        core.validation.push_back(Err(failure("rejected")));
        let mut handler = ConfigCommandHandler::new(&mut store, &mut core);

        let error = handler
            .apply_profile_yaml(&second, "mode: global\n", 2_000)
            .unwrap_err();

        assert_eq!(error.recovery, RecoveryStatus::NotNeeded);
        assert_eq!(store.selected(), Some(&first));
        assert_eq!(store.yaml(&second).unwrap(), "mode: direct\n");
    }

    #[test]
    fn unhealthy_candidate_restores_file_and_previous_runtime() {
        let directory = TestDir::new("health-rollback");
        let (mut store, first, second) = store_with_profiles(&directory.0);
        let mut core = FakeCore::default();
        core.health.push_back(Err(failure("unhealthy")));
        core.health.push_back(Ok(()));
        let mut handler = ConfigCommandHandler::new(&mut store, &mut core);

        let error = handler
            .apply_profile_yaml(&second, "mode: global\n", 2_000)
            .unwrap_err();

        assert_eq!(error.recovery, RecoveryStatus::Restored);
        assert_eq!(store.selected(), Some(&first));
        assert_eq!(store.yaml(&second).unwrap(), "mode: direct\n");
        assert_eq!(core.applied_yaml, ["mode: global\n", "mode: rule\n"]);
    }

    #[test]
    fn recovery_failure_is_reported_separately_from_primary_error() {
        let directory = TestDir::new("failed-recovery");
        let (mut store, _, second) = store_with_profiles(&directory.0);
        let mut core = FakeCore::default();
        core.apply.push_back(Err(failure("apply failed")));
        core.apply.push_back(Err(failure("restore failed")));
        let mut handler = ConfigCommandHandler::new(&mut store, &mut core);

        let error = handler
            .apply_profile_yaml(&second, "mode: global\n", 2_000)
            .unwrap_err();

        assert_eq!(error.cause.message, "apply failed");
        assert!(matches!(error.recovery, RecoveryStatus::Failed { .. }));
        assert_eq!(store.yaml(&second).unwrap(), "mode: direct\n");
    }

    #[test]
    fn failed_first_activation_restores_file_and_stops_core() {
        let directory = TestDir::new("first-activation");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let second = ProfileId::parse("second").unwrap();
        store
            .import(
                Profile::new(
                    second.clone(),
                    "Second",
                    ProfileSource::Local,
                    UpdatePolicy::Manual,
                    1_000,
                    None)
                .unwrap(),
                "mode: direct\n",
            )
            .unwrap();
        let mut core = FakeCore::default();
        core.health.push_back(Err(failure("unhealthy")));
        let mut handler = ConfigCommandHandler::new(&mut store, &mut core);

        let error = handler
            .apply_profile_yaml(&second, "mode: global\n", 2_000)
            .unwrap_err();

        assert_eq!(error.recovery, RecoveryStatus::Restored);
        assert!(store.selected().is_none());
        assert_eq!(store.yaml(&second).unwrap(), "mode: direct\n");
        assert_eq!(core.stop_calls, 1);
    }

    #[derive(Default)]
    struct FakeRuntime {
        mode: Option<RunMode>,
        groups: Vec<ProxyGroup>,
        events: Vec<RealtimeEvent>,
        calls: Vec<String>,
        failure: Option<AppError>,
    }

    impl RuntimeControl for FakeRuntime {
        fn mode(&mut self) -> Result<RunMode, AppError> {
            self.calls.push("mode".into());
            self.failure.take().map_or(Ok(self.mode.unwrap()), Err)
        }

        fn set_mode(&mut self, mode: RunMode) -> Result<(), AppError> {
            self.calls.push(format!("set_mode:{mode:?}"));
            if let Some(error) = self.failure.take() {
                return Err(error);
            }
            self.mode = Some(mode);
            Ok(())
        }

        fn proxy_groups(&mut self) -> Result<Vec<ProxyGroup>, AppError> {
            self.calls.push("proxy_groups".into());
            self.failure
                .take()
                .map_or_else(|| Ok(self.groups.clone()), Err)
        }

        fn rules(&mut self) -> Result<Vec<RuleEntry>, AppError> {
            self.calls.push("rules".into());
            self.failure.take().map_or(Ok(Vec::new()), Err)
        }

        fn providers(&mut self) -> Result<Vec<ProviderSummary>, AppError> {
            self.calls.push("providers".into());
            self.failure.take().map_or(Ok(Vec::new()), Err)
        }

        fn update_provider(&mut self, kind: ProviderKind, name: &str) -> Result<(), AppError> {
            self.calls.push(format!("update_provider:{kind:?}:{name}"));
            self.failure.take().map_or(Ok(()), Err)
        }

        fn network_settings(&mut self) -> Result<NetworkSettings, AppError> {
            self.calls.push("network_settings".into());
            self.failure.take().map_or(
                Ok(NetworkSettings {
                    tun_enabled: false,
                    dns_enabled: true,
                    ipv6_enabled: false,
                }),
                Err,
            )
        }

        fn set_network_settings(&mut self, settings: &NetworkSettings) -> Result<(), AppError> {
            self.calls.push(format!(
                "set_network_settings:{}:{}:{}",
                settings.tun_enabled, settings.dns_enabled, settings.ipv6_enabled
            ));
            self.failure.take().map_or(Ok(()), Err)
        }

        fn select_proxy(&mut self, group: &str, proxy: &str) -> Result<(), AppError> {
            self.calls.push(format!("select:{group}:{proxy}"));
            self.failure.take().map_or(Ok(()), Err)
        }

        fn delay(&mut self, proxy: &str, url: &str, timeout_ms: u32) -> Result<u32, AppError> {
            self.calls.push(format!("delay:{proxy}:{url}:{timeout_ms}"));
            self.failure.take().map_or(Ok(42), Err)
        }

        fn close_connection(&mut self, id: &str) -> Result<(), AppError> {
            self.calls.push(format!("close_connection:{id}"));
            self.failure.take().map_or(Ok(()), Err)
        }

        fn start_realtime(&mut self, topics: &[RealtimeTopic]) -> Result<(), AppError> {
            self.calls.push(format!("start:{topics:?}"));
            self.failure.take().map_or(Ok(()), Err)
        }

        fn stop_realtime(&mut self) {
            self.calls.push("stop".into());
        }

        fn drain_realtime(&mut self) -> Vec<RealtimeEvent> {
            self.calls.push("drain".into());
            std::mem::take(&mut self.events)
        }
    }

    fn runtime() -> FakeRuntime {
        FakeRuntime {
            mode: Some(RunMode::Rule),
            groups: vec![ProxyGroup {
                name: "Select".into(),
                kind: "Selector".into(),
                selected: Some("Node".into()),
                members: vec!["Node".into(), "DIRECT".into()],
            }],
            events: vec![RealtimeEvent::Traffic(verge_domain::TrafficEvent {
                up: 1,
                down: 2,
            })],
            calls: Vec::new(),
            failure: None,
        }
    }

    #[test]
    fn runtime_handler_routes_reads_and_returns_typed_outputs() {
        let mut runtime = runtime();
        let mut handler = RuntimeCommandHandler::new(&mut runtime);

        assert_eq!(
            handler.execute(RuntimeCommand::GetMode).unwrap().output,
            RuntimeCommandOutput::Mode(RunMode::Rule)
        );
        assert!(matches!(
            handler
                .execute(RuntimeCommand::ListProxyGroups)
                .unwrap()
                .output,
            RuntimeCommandOutput::ProxyGroups(groups) if groups.len() == 1
        ));
        assert!(matches!(
            handler.execute(RuntimeCommand::ListRules).unwrap().output,
            RuntimeCommandOutput::Rules(rules) if rules.is_empty()
        ));
        assert!(matches!(
            handler
                .execute(RuntimeCommand::ListProviders)
                .unwrap()
                .output,
            RuntimeCommandOutput::Providers(providers) if providers.is_empty()
        ));
        assert!(matches!(
            handler
                .execute(RuntimeCommand::GetNetworkSettings)
                .unwrap()
                .output,
            RuntimeCommandOutput::NetworkSettings(NetworkSettings {
                dns_enabled: true,
                ..
            })
        ));
        assert!(matches!(
            handler
                .execute(RuntimeCommand::DrainRealtime)
                .unwrap()
                .output,
            RuntimeCommandOutput::Realtime(events) if events.len() == 1
        ));
    }

    #[test]
    fn runtime_handler_routes_all_writes_without_bypassing_control() {
        let mut runtime = runtime();
        let mut handler = RuntimeCommandHandler::new(&mut runtime);

        handler
            .execute(RuntimeCommand::SetMode {
                mode: RunMode::Global,
            })
            .unwrap();
        handler
            .execute(RuntimeCommand::SelectProxy {
                group: "Select".into(),
                proxy: "Node".into(),
            })
            .unwrap();
        assert_eq!(
            handler
                .execute(RuntimeCommand::TestProxyDelay {
                    proxy: "Node".into(),
                    url: "https://example.com/generate_204".into(),
                    timeout_ms: 5_000,
                })
                .unwrap()
                .output,
            RuntimeCommandOutput::Delay(42)
        );
        handler
            .execute(RuntimeCommand::CloseConnection { id: "abc".into() })
            .unwrap();
        handler
            .execute(RuntimeCommand::UpdateProvider {
                kind: ProviderKind::Rule,
                name: "ads".into(),
            })
            .unwrap();
        handler
            .execute(RuntimeCommand::SetNetworkSettings {
                settings: NetworkSettings {
                    tun_enabled: true,
                    dns_enabled: true,
                    ipv6_enabled: false,
                },
            })
            .unwrap();
        assert_eq!(runtime.mode, Some(RunMode::Global));
        assert!(runtime.calls.iter().any(|call| call.starts_with("select:")));
        assert!(runtime.calls.contains(&"close_connection:abc".to_owned()));
        assert!(
            runtime
                .calls
                .contains(&"update_provider:Rule:ads".to_owned())
        );
        assert!(
            runtime
                .calls
                .contains(&"set_network_settings:true:true:false".to_owned())
        );
    }

    #[test]
    fn runtime_handler_owns_realtime_lifecycle() {
        let mut runtime = runtime();
        let mut handler = RuntimeCommandHandler::new(&mut runtime);
        handler
            .execute(RuntimeCommand::StartRealtime {
                topics: vec![RealtimeTopic::Traffic, RealtimeTopic::Logs],
            })
            .unwrap();
        handler.execute(RuntimeCommand::StopRealtime).unwrap();
        assert_eq!(
            runtime.calls,
            ["start:[Traffic, Logs]".to_owned(), "stop".to_owned()]
        );
    }

    #[test]
    fn runtime_handler_preserves_machine_readable_errors() {
        let mut runtime = runtime();
        runtime.failure = Some(AppError::new(ErrorCode::CoreUnavailable, "offline"));
        let mut handler = RuntimeCommandHandler::new(&mut runtime);

        let error = handler.execute(RuntimeCommand::GetMode).unwrap_err();
        assert_eq!(error.code, ErrorCode::CoreUnavailable);
        assert_eq!(error.message, "offline");
    }

    struct FakeSystemProxy {
        state: SystemProxyState,
        calls: Vec<String>,
        failure: Option<AppError>,
    }

    impl SystemProxyControl for FakeSystemProxy {
        fn recovery_pending(&self) -> bool {
            self.state.recovery_pending
        }

        fn state(&mut self) -> Result<SystemProxyState, AppError> {
            self.calls.push("state".into());
            self.failure
                .take()
                .map_or_else(|| Ok(self.state.clone()), Err)
        }

        fn enable(
            &mut self,
            services: &[String],
            endpoint: &ProxyEndpoint,
        ) -> Result<SystemProxyState, AppError> {
            self.calls.push(format!(
                "enable:{services:?}:{}:{}",
                endpoint.host, endpoint.port
            ));
            self.failure
                .take()
                .map_or_else(|| Ok(self.state.clone()), Err)
        }

        fn disable(&mut self) -> Result<SystemProxyState, AppError> {
            self.calls.push("disable".into());
            self.failure
                .take()
                .map_or_else(|| Ok(self.state.clone()), Err)
        }

        fn recover_pending(&mut self) -> Result<SystemProxyState, AppError> {
            self.calls.push("recover".into());
            self.failure
                .take()
                .map_or_else(|| Ok(self.state.clone()), Err)
        }

        fn set_socks(
            &mut self,
            enabled: bool,
            endpoint: &ProxyEndpoint,
        ) -> Result<SystemProxyState, AppError> {
            self.calls.push(format!(
                "set_socks:{enabled}:{}:{}",
                endpoint.host, endpoint.port
            ));
            self.failure
                .take()
                .map_or_else(|| Ok(self.state.clone()), Err)
        }

        fn set_auto_proxy(&mut self, url: Option<&str>) -> Result<SystemProxyState, AppError> {
            self.calls.push(format!("set_auto_proxy:{url:?}"));
            self.failure
                .take()
                .map_or_else(|| Ok(self.state.clone()), Err)
        }

        fn set_proxy_bypass(&mut self, domains: &[String]) -> Result<SystemProxyState, AppError> {
            self.calls.push(format!("set_proxy_bypass:{domains:?}"));
            self.failure
                .take()
                .map_or_else(|| Ok(self.state.clone()), Err)
        }
    }

    fn fake_system_proxy() -> FakeSystemProxy {
        FakeSystemProxy {
            state: SystemProxyState {
                services: Vec::new(),
                recovery_pending: false,
            },
            calls: Vec::new(),
            failure: None,
        }
    }

    #[test]
    fn lifecycle_recovers_proxy_before_starting_core_and_realtime() {
        let mut core = FakeCore::default();
        let mut runtime = runtime();
        let mut proxy = fake_system_proxy();
        proxy.state.recovery_pending = true;

        ApplicationLifecycle::new(&mut core, &mut runtime, &mut proxy)
            .start()
            .unwrap();

        assert_eq!(proxy.calls, ["recover"]);
        assert_eq!(core.start_calls, 1);
        assert_eq!(core.stop_calls, 0);
        assert_eq!(
            runtime.calls,
            ["start:[Traffic, Memory, Connections, Logs]"]
        );
    }

    #[test]
    fn lifecycle_stops_core_when_health_check_fails() {
        let mut core = FakeCore {
            health: VecDeque::from([Err(failure("unhealthy"))]),
            ..FakeCore::default()
        };
        let mut runtime = runtime();
        let mut proxy = fake_system_proxy();

        let error = ApplicationLifecycle::new(&mut core, &mut runtime, &mut proxy)
            .start()
            .unwrap_err();

        assert_eq!(error.cause.message, "unhealthy");
        assert_eq!(core.start_calls, 1);
        assert_eq!(core.stop_calls, 1);
        assert!(runtime.calls.is_empty());
    }

    #[test]
    fn lifecycle_rolls_back_runtime_and_core_when_subscription_start_fails() {
        let mut core = FakeCore::default();
        let mut runtime = runtime();
        runtime.failure = Some(failure("websocket unavailable"));
        let mut proxy = fake_system_proxy();

        let error = ApplicationLifecycle::new(&mut core, &mut runtime, &mut proxy)
            .start()
            .unwrap_err();

        assert_eq!(error.cause.message, "websocket unavailable");
        assert_eq!(runtime.calls.last().map(String::as_str), Some("stop"));
        assert_eq!(core.stop_calls, 1);
    }

    #[test]
    fn lifecycle_shutdown_stops_realtime_recovers_proxy_and_stops_core() {
        let mut core = FakeCore::default();
        let mut runtime = runtime();
        let mut proxy = fake_system_proxy();
        proxy.state.recovery_pending = true;

        ApplicationLifecycle::new(&mut core, &mut runtime, &mut proxy)
            .shutdown()
            .unwrap();

        assert_eq!(runtime.calls, ["stop"]);
        assert_eq!(proxy.calls, ["recover"]);
        assert_eq!(core.stop_calls, 1);
    }

    #[test]
    fn system_proxy_handler_routes_state_and_privileged_writes() {
        let mut proxy = fake_system_proxy();
        let mut handler = SystemProxyCommandHandler::new(&mut proxy);
        handler.execute(SystemProxyCommand::GetState).unwrap();
        handler
            .execute(SystemProxyCommand::Enable {
                services: vec!["Wi-Fi".into()],
                endpoint: ProxyEndpoint::new("127.0.0.1", 7890).unwrap(),
            })
            .unwrap();
        handler.execute(SystemProxyCommand::Disable).unwrap();
        handler.execute(SystemProxyCommand::RecoverPending).unwrap();
        assert_eq!(
            proxy.calls,
            [
                "state",
                "enable:[\"Wi-Fi\"]:127.0.0.1:7890",
                "disable",
                "recover"
            ]
        );
    }

    #[test]
    fn system_proxy_handler_routes_socks_pac_and_bypass_writes() {
        let mut proxy = fake_system_proxy();
        let mut handler = SystemProxyCommandHandler::new(&mut proxy);
        handler
            .execute(SystemProxyCommand::SetSocks {
                enabled: true,
                endpoint: ProxyEndpoint::new("127.0.0.1", 7891).unwrap(),
            })
            .unwrap();
        handler
            .execute(SystemProxyCommand::SetAutoProxy {
                url: Some("http://127.0.0.1/proxy.pac".into()),
            })
            .unwrap();
        handler
            .execute(SystemProxyCommand::SetProxyBypass {
                domains: vec!["*.local".into()],
            })
            .unwrap();
        assert_eq!(
            proxy.calls,
            [
                "set_socks:true:127.0.0.1:7891",
                "set_auto_proxy:Some(\"http://127.0.0.1/proxy.pac\")",
                "set_proxy_bypass:[\"*.local\"]"
            ]
        );
    }

    #[test]
    fn system_proxy_handler_preserves_platform_errors() {
        let mut proxy = fake_system_proxy();
        proxy.failure = Some(AppError::new(
            ErrorCode::PlatformFailed,
            "permission denied",
        ));
        let mut handler = SystemProxyCommandHandler::new(&mut proxy);
        let error = handler.execute(SystemProxyCommand::Disable).unwrap_err();
        assert_eq!(error.code, ErrorCode::PlatformFailed);
    }
}

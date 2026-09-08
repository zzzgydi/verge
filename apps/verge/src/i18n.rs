//! 手写查表 i18n：不引第三方依赖。
//!
//! `ENTRIES` 是唯一的文案表，每行 `(key, zh-CN, en)`，两语言 key 集合
//! 由构造保证一致（测试再强制 key 唯一、两语言非空）。`tr` 线性查找，
//! 文案量级小、渲染开销可忽略。带参数的文案用本模块的 `fmt_*` 函数，
//! 保持中英文语序/标点差异集中在表和函数里，调用点不拼句子。

/// 界面语言。`ApplicationSettings.language` 已校验只可能是 `"zh-CN"` / `"en"`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lang {
    ZhCn,
    En,
}

impl Lang {
    /// 设置里的语言码到枚举的映射；未加载设置或未知值回退英文（与 domain 默认值一致）。
    pub fn from_code(code: &str) -> Self {
        match code {
            "zh-CN" => Self::ZhCn,
            _ => Self::En,
        }
    }
}

/// 文案表：`(key, zh-CN, en)`。key 按页面/用途命名。
const ENTRIES: &[(&str, &str, &str)] = &[
    ("network.saved", "网络设置已保存", "Network settings saved"),
    ("network.lan", "局域网连接", "Allow LAN"),
    (
        "network.lan.desc",
        "允许同一网络中的设备使用此代理",
        "Allow devices on your network to use this proxy",
    ),
    (
        "network.ipv6.desc",
        "允许 IPv6 连接与解析",
        "Enable IPv6 connections and resolution",
    ),
    ("network.delay", "统一延迟", "Unified delay"),
    (
        "network.delay.desc",
        "使用统一方式计算节点延迟",
        "Use a consistent method to measure proxy latency",
    ),
    ("network.dns", "DNS 覆写", "DNS override"),
    (
        "network.dns.desc",
        "关闭后使用订阅配置中的 DNS 设置",
        "When off, use DNS settings from the profile",
    ),
    (
        "network.dns.title",
        "DNS 覆写配置",
        "DNS override configuration",
    ),
    ("network.log", "日志等级", "Log level"),
    ("network.port", "代理端口", "Proxy port"),
    (
        "network.port.desc",
        "HTTP 与 SOCKS 共用端口",
        "Shared port for HTTP and SOCKS",
    ),
    (
        "network.port.invalid",
        "端口必须在 1–65535 之间",
        "Port must be between 1 and 65535",
    ),
    ("network.controller", "外部控制器", "External controller"),
    (
        "network.controller.desc",
        "供外部面板或 API 客户端访问",
        "Access from external dashboards and API clients",
    ),
    (
        "network.controller.enable",
        "启用外部控制器",
        "Enable external controller",
    ),
    ("network.controller.address", "监听地址", "Listen address"),
    ("network.controller.secret", "API 访问密钥", "API secret"),
    ("network.disabled", "已关闭", "Disabled"),
    (
        "home.subtitle",
        "网络状态，尽在眼前。",
        "Your network, at a glance.",
    ),
    ("home.traffic", "实时流量", "Network activity"),
    ("home.samples", "最近 40 次采样", "LAST 40 SAMPLES"),
    (
        "home.traffic_waiting",
        "内核启动后显示实时流量",
        "Traffic appears when the core is running",
    ),
    ("home.recent", "较早", "Earlier"),
    ("home.now", "现在", "Now"),
    ("home.core_memory", "Mihomo 内存", "Mihomo memory"),
    ("home.active_profile", "当前配置", "Active profile"),
    ("home.no_profile", "尚未选择配置", "No profile selected"),
    (
        "home.profile_hint",
        "导入并启用配置，开始连接。",
        "Import and activate a profile to get connected.",
    ),
    ("home.manage_profiles", "管理配置", "Manage profiles"),
    ("home.quick_access", "工作区", "Workspace"),
    ("home.proxy_hint", "查看节点与延迟", "Nodes & latency"),
    ("home.rules_hint", "查看流量路由规则", "Routing & providers"),
    ("home.logs_hint", "查看运行日志", "Events & diagnostics"),
    // ---- 通用 ----
    ("common.refresh", "刷新", "Refresh"),
    ("common.cancel", "取消", "Cancel"),
    ("common.save", "保存", "Save"),
    ("common.delete", "删除", "Delete"),
    ("common.copy", "复制", "Copy"),
    ("common.copied", "已复制到剪贴板", "Copied to clipboard"),
    ("common.enable", "启用", "Enable"),
    ("common.disable", "关闭", "Disable"),
    ("common.update_now", "立即更新", "Update Now"),
    ("common.more", "更多…", "More…"),
    ("common.unknown", "未知", "Unknown"),
    // ---- macOS 应用菜单 ----
    ("menu.about", "关于 Verge", "About Verge"),
    ("menu.services", "服务", "Services"),
    ("menu.hide", "隐藏 Verge", "Hide Verge"),
    ("menu.hide_others", "隐藏其他", "Hide Others"),
    ("menu.show_all", "全部显示", "Show All"),
    ("menu.quit", "退出 Verge", "Quit Verge"),
    ("menu.file", "文件", "File"),
    ("menu.close_window", "关闭窗口", "Close Window"),
    ("menu.edit", "编辑", "Edit"),
    ("menu.undo", "撤销", "Undo"),
    ("menu.redo", "重做", "Redo"),
    ("menu.cut", "剪切", "Cut"),
    ("menu.copy", "复制", "Copy"),
    ("menu.paste", "粘贴", "Paste"),
    ("menu.select_all", "全选", "Select All"),
    ("menu.window", "窗口", "Window"),
    ("menu.minimize", "最小化", "Minimize"),
    ("menu.zoom", "缩放", "Zoom"),
    ("menu.full_screen", "进入全屏幕", "Enter Full Screen"),
    ("menu.help", "帮助", "Help"),
    ("menu.project_page", "Verge 项目主页", "Verge Project Page"),
    // ---- 侧边栏 / 页面标题 ----
    ("nav.header", "导航", "Navigation"),
    ("nav.group.proxy", "代理", "Proxy"),
    ("nav.group.system", "系统", "System"),
    (
        "settings.subtitle",
        "让 Verge 适合你的使用习惯。",
        "Make Verge work the way you do.",
    ),
    (
        "proxies.subtitle",
        "选择流量模式与代理节点。",
        "Choose how traffic is routed and where it connects.",
    ),
    (
        "rules.subtitle",
        "查看流量匹配规则与订阅资源。",
        "Inspect routing rules and subscription resources.",
    ),
    (
        "profiles.subtitle",
        "管理订阅、本地配置与合并规则。",
        "Manage subscriptions, local profiles and merge rules.",
    ),
    ("rules.providers", "订阅资源", "Providers"),
    ("rules.column.type", "类型", "Type"),
    ("rules.column.match", "匹配内容", "Match"),
    ("rules.column.target", "目标策略", "Policy"),
    (
        "connections.subtitle",
        "查看活跃连接与实时用量。",
        "Monitor active connections and live usage.",
    ),
    (
        "logs.subtitle",
        "按级别查看内核运行记录。",
        "Inspect core activity by log level.",
    ),
    ("home.title", "概览", "Overview"),
    ("proxies.title", "代理", "Proxies"),
    ("rules.title", "规则", "Rules"),
    ("connections.title", "连接", "Connections"),
    ("profiles.title", "配置", "Profiles"),
    ("logs.title", "日志", "Logs"),
    ("settings.title", "设置", "Settings"),
    // ---- 内核 / 模式状态 ----
    ("status.core_running", "内核运行中", "Core running"),
    ("status.core_offline", "内核离线", "Core offline"),
    ("status.core_unknown", "状态未知", "Status unknown"),
    (
        "status.core_state_unknown",
        "内核状态未知",
        "Core status unknown",
    ),
    ("status.mode_unknown", "模式未知", "Mode unknown"),
    (
        "status.connections_waiting",
        "连接等待中",
        "Waiting for connections…",
    ),
    ("home.mode.rule", "规则", "Rule"),
    ("home.mode.global", "全局", "Global"),
    ("home.mode.direct", "直连", "Direct"),
    // ---- 标题栏 ----
    (
        "titlebar.theme.system",
        "主题：跟随系统（点击切换）",
        "Theme: System (click to change)",
    ),
    (
        "titlebar.theme.light",
        "主题：浅色（点击切换）",
        "Theme: Light (click to change)",
    ),
    (
        "titlebar.theme.dark",
        "主题：深色（点击切换）",
        "Theme: Dark (click to change)",
    ),
    // ---- 概览页 ----
    ("home.core.running", "运行中", "Running"),
    ("home.core.offline", "离线", "Offline"),
    ("home.tile.core", "内核状态", "Core Status"),
    ("home.tile.upload", "上传速率", "Upload"),
    ("home.tile.download", "下载速率", "Download"),
    ("home.tile.memory", "内存占用", "Memory"),
    ("home.tile.connections", "连接数", "Connections"),
    ("home.proxy_control", "代理控制", "Proxy Control"),
    ("home.run_mode", "运行模式", "Mode"),
    ("home.system_proxy", "系统代理", "System Proxy"),
    // ---- 代理页 ----
    (
        "proxies.search",
        "筛选代理名称、分组或协议",
        "Filter names, groups or protocols",
    ),
    ("proxies.collapse_all", "全部收起", "Collapse all"),
    ("proxies.locate", "定位选中", "Locate selected"),
    ("proxies.timeout", "超时", "Timeout"),
    ("proxies.direct.title", "直连模式", "Direct mode"),
    (
        "proxies.direct.desc",
        "流量直接连接目标。切换规则或全局模式以选择代理。",
        "Traffic connects directly. Switch to Rule or Global to select proxies.",
    ),
    (
        "proxies.no_match",
        "没有可显示的代理",
        "No proxies to display",
    ),
    (
        "proxies.no_match.desc",
        "尝试清除筛选，或启用配置后刷新。",
        "Clear the filter, or activate a profile and refresh.",
    ),
    ("proxies.test_delay", "测速", "Test"),
    ("proxies.testing", "测速中…", "Testing…"),
    ("proxies.empty.title", "暂无代理组", "No Proxy Groups"),
    (
        "proxies.empty.desc",
        "启用配置后，这里会显示代理组和节点。",
        "Activate a profile to see proxy groups and nodes here.",
    ),
    ("proxies.empty.action", "前往配置", "Go to Profiles"),
    // ---- 连接页 ----
    ("connections.col.process", "进程", "Process"),
    ("connections.col.target", "目标", "Target"),
    ("connections.col.rule", "规则", "Rule"),
    ("connections.col.chains", "链路", "Chains"),
    ("connections.col.upload", "上传", "Upload"),
    ("connections.col.download", "下载", "Download"),
    ("connections.col.actions", "操作", "Actions"),
    ("connections.unknown_process", "未知进程", "Unknown process"),
    ("connections.direct", "直连", "Direct"),
    ("connections.close", "关闭", "Close"),
    ("connections.close_menu", "关闭连接", "Close Connection"),
    (
        "connections.waiting",
        "等待连接快照…",
        "Waiting for connection snapshot…",
    ),
    // ---- 日志页 ----
    ("logs.level.all", "全部", "All"),
    ("logs.level.info", "信息", "Info"),
    ("logs.level.warning", "警告", "Warning"),
    ("logs.level.error", "错误", "Error"),
    ("logs.level.debug", "调试", "Debug"),
    ("logs.empty.title", "没有符合条件的日志", "No Matching Logs"),
    (
        "logs.empty.filtered",
        "当前级别过滤下暂无日志，可清除过滤或等待新日志。",
        "No logs at this level. Clear the filter or wait for new logs.",
    ),
    (
        "logs.empty.unfiltered",
        "内核产生日志后会显示在这里。",
        "Core logs will appear here once available.",
    ),
    ("logs.clear_filter", "清除过滤", "Clear Filter"),
    // ---- 配置页 ----
    ("profiles.policy.manual", "手动更新", "Manual update"),
    ("profiles.policy.unscheduled", "未安排", "not scheduled"),
    ("profiles.policy.next", "下次更新", "next update"),
    (
        "profiles.policy.failures",
        "连续失败",
        "consecutive failures",
    ),
    ("profiles.source.local", "本地", "Local"),
    ("profiles.source.remote", "远程", "Remote"),
    ("profiles.selected", "已启用", "Active"),
    ("profiles.select", "启用", "Activate"),
    ("profiles.view_yaml", "查看 YAML…", "View YAML…"),
    ("profiles.merged", "合并结果…", "Merged Result…"),
    ("profiles.set_interval", "设置间隔…", "Set Interval…"),
    ("profiles.delete_menu", "删除配置…", "Delete Profile…"),
    ("profiles.current", "当前", "Current"),
    ("profiles.merge_config", "Merge 配置…", "Merge Config…"),
    ("profiles.import", "导入配置…", "Import Profile…"),
    ("profiles.empty.title", "暂无配置", "No Profiles"),
    (
        "profiles.empty.desc",
        "导入本地 YAML 或远程订阅后开始使用。",
        "Import a local YAML or a remote subscription to get started.",
    ),
    // ---- 规则页 ----
    ("rules.providers.empty", "暂无 Provider。", "No providers."),
    ("rules.provider_kind.proxy", "代理", "Proxy"),
    ("rules.provider_kind.rule", "规则", "Rule"),
    ("rules.update", "更新", "Update"),
    ("rules.empty.title", "暂无规则", "No Rules"),
    (
        "rules.empty.desc",
        "启用配置后点击“刷新”加载规则列表。",
        "Activate a profile, then click \"Refresh\" to load rules.",
    ),
    ("rules.section", "规则", "Rules"),
    // ---- 设置页 ----
    ("settings.group.general", "通用", "General"),
    ("settings.group.network", "网络", "Network"),
    ("settings.group.proxy", "系统代理", "System Proxy"),
    ("settings.group.core", "Mihomo 内核", "Mihomo Core"),
    ("settings.group.app_update", "应用更新", "App Update"),
    ("settings.group.system", "系统", "System"),
    ("settings.group.backup", "加密备份", "Encrypted Backup"),
    ("settings.theme", "主题", "Theme"),
    ("settings.theme.system", "跟随系统", "System"),
    ("settings.theme.light", "浅色", "Light"),
    ("settings.theme.dark", "深色", "Dark"),
    ("settings.language", "语言", "Language"),
    ("settings.log_limit", "日志缓冲", "Log Buffer"),
    (
        "settings.log_limit.desc",
        "100 – 5000 条，超出后丢弃最旧的日志",
        "Keep 100 – 5000 entries; the oldest are dropped beyond the limit",
    ),
    ("settings.launch_at_login", "开机启动", "Launch at Login"),
    (
        "settings.launch_at_login.desc",
        "登录 macOS 后自动启动 Verge，需打包为 .app 才能生效",
        "Start Verge automatically after signing in to macOS; requires the packaged .app",
    ),
    ("settings.global_hotkey", "全局快捷键", "Global Hotkey"),
    (
        "settings.global_hotkey.desc",
        "显示/隐藏主窗口；留空保存即禁用，组合键被占用时会回滚到旧快捷键",
        "Show/hide the main window. Save empty to disable; if the shortcut is taken, the previous one is restored",
    ),
    ("settings.reset_default", "恢复默认", "Reset to Defaults"),
    (
        "settings.reset_default.desc",
        "按作用域恢复默认值，不影响配置和备份",
        "Reset one scope to defaults; profiles and backups are not affected",
    ),
    ("settings.scope.appearance", "外观", "Appearance"),
    ("settings.scope.network", "网络", "Network"),
    ("settings.scope.system", "系统", "System"),
    ("settings.network.tun", "TUN 模式", "TUN Mode"),
    (
        "settings.network.not_loaded",
        "网络设置尚未加载。",
        "Network settings not loaded yet.",
    ),
    ("settings.proxy.socks", "SOCKS 代理", "SOCKS Proxy"),
    (
        "settings.proxy.socks.unavailable",
        "配置未声明 mixed-port 或 socks-port，无法启用",
        "The profile does not declare mixed-port or socks-port; cannot enable",
    ),
    (
        "settings.proxy.socks.available",
        "使用配置的 mixed-port 或 socks-port",
        "Uses the profile's mixed-port or socks-port",
    ),
    ("settings.proxy.pac", "自动代理（PAC）", "Auto Proxy (PAC)"),
    ("settings.proxy.pac.enabled", "已启用", "Enabled"),
    ("settings.proxy.not_set", "未设置", "Not set"),
    ("settings.proxy.bypass", "代理绕过列表", "Proxy Bypass List"),
    (
        "settings.proxy.bypass.empty",
        "未设置；输入框留空保存即清空",
        "Not set; save with an empty input to clear",
    ),
    (
        "settings.proxy.not_loaded",
        "系统代理状态尚未加载。",
        "System proxy state not loaded yet.",
    ),
    ("settings.core.update", "内核更新", "Core Update"),
    (
        "settings.core.update.desc",
        "下载、校验并更新 Mihomo 内核",
        "Download, verify, and update the Mihomo core",
    ),
    ("settings.core.version", "当前版本", "Current Version"),
    (
        "settings.app.version_dev",
        "未打包运行（开发模式），应用更新不可用",
        "Running unpackaged (dev mode); app updates are unavailable",
    ),
    ("settings.app.check", "检查更新", "Check for Updates"),
    (
        "settings.app.check.desc",
        "从 GitHub Releases 检查最新版本",
        "Check GitHub Releases for the latest version",
    ),
    ("settings.app.latest", "最新版本", "Latest Version"),
    ("settings.app.update", "更新", "Update"),
    (
        "settings.app.update.download",
        "下载并更新",
        "Download & Update",
    ),
    ("settings.app.pending_restart", "待重启", "Pending Restart"),
    ("settings.app.restart", "重启应用", "Restart App"),
    ("settings.system.data_dir", "数据目录", "Data Directory"),
    ("settings.system.helper", "特权 Helper", "Privileged Helper"),
    (
        "settings.system.helper.install",
        "安装/修复",
        "Install/Repair",
    ),
    ("settings.system.helper.uninstall", "卸载", "Uninstall"),
    ("settings.helper.not_installed", "未安装", "Not installed"),
    (
        "settings.system.diagnostics",
        "诊断导出",
        "Diagnostics Export",
    ),
    (
        "settings.system.diagnostics.export",
        "导出脱敏诊断",
        "Export Redacted Diagnostics",
    ),
    (
        "settings.system.settings_export",
        "设置导出",
        "Settings Export",
    ),
    (
        "settings.system.settings_export.desc",
        "明文设置文件，不含密钥和订阅凭据，可跨机器迁移",
        "Plain-text settings file without keys or subscription credentials; suitable for migration",
    ),
    (
        "settings.system.settings_export.button",
        "导出设置",
        "Export Settings",
    ),
    (
        "settings.system.settings_import",
        "设置导入",
        "Settings Import",
    ),
    (
        "settings.system.settings_import.desc",
        "先预览字段差异，确认后才应用",
        "Preview field differences first; applied only after confirmation",
    ),
    (
        "settings.system.settings_import.button",
        "预览差异并导入",
        "Preview & Import",
    ),
    (
        "settings.backup.passphrase",
        "备份口令",
        "Backup Passphrase",
    ),
    (
        "settings.backup.passphrase.desc",
        "导出和恢复使用同一个口令，至少 12 个字符",
        "The same passphrase is used for export and restore; at least 12 characters",
    ),
    ("settings.backup.actions", "备份操作", "Backup Actions"),
    (
        "settings.backup.export",
        "导出加密备份",
        "Export Encrypted Backup",
    ),
    ("settings.backup.restore", "恢复备份", "Restore Backup"),
    // ---- 输入框占位 ----
    (
        "placeholder.profile_id",
        "配置 ID（如 daily）",
        "Profile ID (e.g. daily)",
    ),
    ("placeholder.profile_name", "显示名称", "Display name"),
    (
        "placeholder.profile_interval",
        "更新间隔（秒）",
        "Update interval (seconds)",
    ),
    (
        "placeholder.profile_user_agent",
        "可选，如 ClashX/1.0（留空用默认）",
        "Optional, e.g. ClashX/1.0 (empty for default)",
    ),
    (
        "placeholder.backup_passphrase",
        "备份口令（至少 12 个字符）",
        "Backup passphrase (at least 12 characters)",
    ),
    (
        "placeholder.settings_import_path",
        "设置导出文件的绝对路径",
        "Absolute path of the settings export file",
    ),
    (
        "placeholder.proxy_bypass",
        "以逗号分隔，如 *.local, 192.168.0.0/16",
        "Comma-separated, e.g. *.local, 192.168.0.0/16",
    ),
    (
        "placeholder.global_hotkey",
        "如 CmdOrCtrl+Shift+V，留空即禁用",
        "e.g. CmdOrCtrl+Shift+V; empty to disable",
    ),
    // ---- 对话框 / Sheet ----
    ("dialog.import.title", "导入配置", "Import Profile"),
    ("dialog.import.id", "配置 ID", "Profile ID"),
    ("dialog.import.name", "显示名称", "Display Name"),
    ("dialog.import.url", "订阅地址", "Subscription URL"),
    (
        "dialog.import.url.desc",
        "填写订阅地址后可作为远程配置导入，按间隔自动更新",
        "Fill in a subscription URL to import as a remote profile with automatic updates",
    ),
    (
        "dialog.import.interval",
        "更新间隔（秒）",
        "Update Interval (seconds)",
    ),
    (
        "dialog.import.user_agent.desc",
        "订阅下载请求的 UA，留空使用默认值",
        "User-Agent for subscription downloads; empty for default",
    ),
    ("dialog.import.yaml", "本地 YAML 内容", "Local YAML Content"),
    (
        "dialog.import.local",
        "导入本地配置",
        "Import Local Profile",
    ),
    (
        "dialog.import.remote",
        "导入远程配置",
        "Import Remote Profile",
    ),
    ("dialog.interval.title", "更新间隔", "Update Interval"),
    (
        "dialog.interval.desc",
        "仅对远程配置生效",
        "Only applies to remote profiles",
    ),
    (
        "dialog.delete_profile.desc",
        "该配置的本地文件将被移除，此操作不可恢复。",
        "The local files of this profile will be removed. This cannot be undone.",
    ),
    (
        "dialog.restore_backup.title",
        "恢复加密备份？",
        "Restore Encrypted Backup?",
    ),
    (
        "dialog.restore_backup.desc",
        "现有配置将被备份内容覆盖，此操作不可恢复。",
        "Existing profiles will be overwritten by the backup. This cannot be undone.",
    ),
    ("dialog.restore_backup.ok", "恢复", "Restore"),
    (
        "dialog.uninstall_helper.title",
        "卸载特权 Helper？",
        "Uninstall Privileged Helper?",
    ),
    (
        "dialog.uninstall_helper.desc",
        "将移除 LaunchDaemon 和特权 helper 程序，TUN 模式随即不可用；之后可在本页重新安装。",
        "Removes the LaunchDaemon and the privileged helper; TUN mode will stop working. You can reinstall it on this page later.",
    ),
    (
        "dialog.update_app.desc",
        "将下载、校验并替换当前 Verge.app，更新在重启后生效；替换失败会自动还原现有安装。",
        "Downloads, verifies, and replaces the current Verge.app. The update takes effect after restart; a failed replacement automatically restores the current install.",
    ),
    ("dialog.restart.title", "重启 Verge？", "Restart Verge?"),
    (
        "dialog.restart.desc",
        "应用将立即退出并以新版本启动；代理内核会先停止再随新实例恢复。",
        "The app quits immediately and relaunches on the new version; the proxy core stops first and resumes with the new instance.",
    ),
    ("dialog.restart.ok", "重启", "Restart"),
    (
        "dialog.reset_scope.desc",
        "只重置该作用域的设置字段，配置、备份和其它设置不受影响。",
        "Only settings in this scope are reset; profiles, backups, and other settings are not affected.",
    ),
    ("dialog.reset_scope.ok", "恢复默认", "Reset"),
    (
        "dialog.import_settings.title",
        "导入设置",
        "Import Settings",
    ),
    (
        "dialog.import_settings.no_changes",
        "与当前设置一致，导入后没有字段变化。",
        "Identical to current settings; importing changes nothing.",
    ),
    (
        "dialog.import_settings.field_diff",
        "字段差异",
        "Field Differences",
    ),
    ("dialog.import_settings.ok", "应用导入", "Apply Import"),
    (
        "settings_import.empty_path",
        "请先填写设置文件的绝对路径",
        "Enter the absolute path of the settings file first",
    ),
    (
        "sheet.yaml.loading",
        "正在加载配置 YAML…",
        "Loading profile YAML…",
    ),
    ("sheet.yaml.title", "配置 YAML", "Profile YAML"),
    (
        "sheet.yaml.load_failed",
        "无法加载配置 YAML",
        "Failed to load profile YAML",
    ),
    ("sheet.yaml.save", "保存到该配置", "Save to This Profile"),
    (
        "sheet.yaml.confirm_desc",
        "编辑器内容将覆盖该配置的现有 YAML。",
        "The editor content will overwrite this profile's existing YAML.",
    ),
    (
        "sheet.merge.loading",
        "正在加载 Merge 配置…",
        "Loading merge config…",
    ),
    (
        "sheet.merge.load_failed",
        "无法加载 Merge 配置",
        "Failed to load merge config",
    ),
    (
        "sheet.merge.title",
        "全局 Merge 配置",
        "Global Merge Config",
    ),
    (
        "sheet.merge.desc",
        "对激活配置的顶层键做受控合并：override / merge / prepend / append / remove。",
        "Controlled merge of top-level keys onto the active profile: override / merge / prepend / append / remove.",
    ),
    ("sheet.merge.save", "保存 Merge 配置", "Save Merge Config"),
    (
        "sheet.merge.confirm_title",
        "保存 Merge 配置？",
        "Save Merge Config?",
    ),
    (
        "sheet.merge.confirm_desc",
        "将立即应用到当前激活配置，校验或健康检查失败会自动回退。",
        "Applied to the active profile immediately; rolls back automatically if validation or the health check fails.",
    ),
    (
        "sheet.merged.loading",
        "正在生成合并结果…",
        "Generating merged result…",
    ),
    ("sheet.merged.title", "合并结果", "Merged Result"),
    (
        "sheet.merged.load_failed",
        "无法生成合并结果",
        "Failed to generate merged result",
    ),
    // ---- 全局错误条 ----
    ("alert.op_failed", "操作未完成", "Operation failed"),
    // ---- Toast / OS 通知 ----
    (
        "notify.profile_done",
        "配置操作已完成",
        "Profile operation completed",
    ),
    (
        "notify.diagnostics_exported",
        "脱敏诊断已导出",
        "Redacted diagnostics exported",
    ),
    (
        "hint.invalid_input",
        "请检查输入后重试",
        "Check your input and try again",
    ),
    (
        "hint.not_found",
        "目标可能已被移除，请刷新后重试",
        "The target may have been removed; refresh and try again",
    ),
    (
        "hint.conflict",
        "请刷新确认当前状态后重试",
        "Refresh to confirm the current state, then try again",
    ),
    (
        "hint.permission_denied",
        "该操作需要明确确认后才能执行",
        "This operation requires explicit confirmation",
    ),
    (
        "hint.core_unavailable",
        "无法连接 Mihomo，请检查内核运行状态和日志",
        "Cannot connect to Mihomo; check the core status and logs",
    ),
    (
        "hint.request_timeout",
        "请求超时，请稍后重试",
        "Request timed out; try again shortly",
    ),
    (
        "hint.proxy_delay_failed",
        "检查节点或测速地址后重试",
        "Check the proxy or test URL and try again",
    ),
    ("proxies.delay_failed", "失败", "Failed"),
    (
        "proxies.delay_retry",
        "点击重试测速",
        "Click to retry delay test",
    ),
    (
        "hint.core_rejected",
        "请检查配置 YAML 后重试",
        "Check the profile YAML and try again",
    ),
    ("toast.profile_imported", "配置已导入", "Profile imported"),
    ("toast.profile_selected", "已启用配置", "Profile activated"),
    ("toast.yaml_saved", "配置 YAML 已保存", "Profile YAML saved"),
    (
        "toast.merge_saved",
        "Merge 配置已保存",
        "Merge config saved",
    ),
    (
        "toast.remote_updated",
        "远程配置已更新",
        "Remote profile updated",
    ),
    (
        "toast.policy_saved",
        "更新策略已保存",
        "Update policy saved",
    ),
    ("toast.profile_deleted", "配置已删除", "Profile deleted"),
    (
        "toast.diagnostics_exported",
        "诊断已导出",
        "Diagnostics exported",
    ),
    ("toast.settings_exported", "设置已导出", "Settings exported"),
    ("toast.settings_imported", "设置已导入", "Settings imported"),
    (
        "toast.settings_reset",
        "已恢复默认设置",
        "Settings reset to defaults",
    ),
    (
        "toast.backup_exported",
        "加密备份已导出",
        "Encrypted backup exported",
    ),
    ("toast.backup_restored", "备份已恢复", "Backup restored"),
    (
        "toast.mihomo_updated",
        "Mihomo 内核已更新",
        "Mihomo core updated",
    ),
    (
        "toast.app_updated",
        "应用更新已就绪，重启后生效",
        "Update ready; takes effect after restart",
    ),
    (
        "toast.app_restarting",
        "正在重启 Verge…",
        "Restarting Verge…",
    ),
    (
        "toast.provider_updated",
        "Provider 已更新",
        "Provider updated",
    ),
];

/// 查表：缺失 key 在 debug 构建下直接 panic（测试会兜住），release 下回退 key 本身。
pub fn tr(lang: Lang, key: &'static str) -> &'static str {
    let Some((_, zh, en)) = ENTRIES.iter().find(|(k, _, _)| *k == key) else {
        debug_assert!(false, "missing i18n key: {key}");
        return key;
    };
    match lang {
        Lang::ZhCn => zh,
        Lang::En => en,
    }
}

/// “标题 · 名称”式的对话框/Sheet 标题（两种语言结构一致）。
pub fn fmt_titled(lang: Lang, key: &'static str, name: &str) -> String {
    format!("{} · {name}", tr(lang, key))
}

/// “当前：xxx”式的状态描述。
pub fn fmt_current(lang: Lang, value: &str) -> String {
    match lang {
        Lang::ZhCn => format!("当前：{value}"),
        Lang::En => format!("Current: {value}"),
    }
}

/// 删除配置确认弹窗标题。
pub fn fmt_delete_profile_title(lang: Lang, name: &str) -> String {
    match lang {
        Lang::ZhCn => format!("删除“{name}”？"),
        Lang::En => format!("Delete \"{name}\"?"),
    }
}

/// YAML Sheet 保存确认弹窗标题。
pub fn fmt_save_yaml_title(lang: Lang, id: &str) -> String {
    match lang {
        Lang::ZhCn => format!("保存到“{id}”？"),
        Lang::En => format!("Save to \"{id}\"?"),
    }
}

/// 应用更新确认弹窗标题。
pub fn fmt_update_app_title(lang: Lang, version: &str) -> String {
    match lang {
        Lang::ZhCn => format!("更新到 v{version}？"),
        Lang::En => format!("Update to v{version}?"),
    }
}

/// 恢复默认值确认弹窗标题（中英文语序不同）。
pub fn fmt_reset_scope_title(lang: Lang, scope_label: &str) -> String {
    match lang {
        Lang::ZhCn => format!("恢复{scope_label}默认值？"),
        Lang::En => format!("Reset {scope_label} to defaults?"),
    }
}

/// Sheet 加载失败的内联错误（冒号后换行接后端英文 message）。
pub fn fmt_load_failed(lang: Lang, key: &'static str, message: &str) -> String {
    match lang {
        Lang::ZhCn => format!("{}：\n{message}", tr(lang, key)),
        Lang::En => format!("{}:\n{message}", tr(lang, key)),
    }
}

/// 设置导入预览的字段差异行。
pub fn fmt_field_change(lang: Lang, field: &str, old: &str, new: &str) -> String {
    match lang {
        Lang::ZhCn => format!("{field}：{old} → {new}"),
        Lang::En => format!("{field}: {old} → {new}"),
    }
}

/// 状态栏连接数。
pub fn fmt_statusbar_connections(lang: Lang, count: usize) -> String {
    match lang {
        Lang::ZhCn => format!("连接 {count}"),
        Lang::En => format!("{count} connections"),
    }
}

/// 连接页汇总行。
pub fn fmt_connections_summary(lang: Lang, count: usize, upload: &str, download: &str) -> String {
    match lang {
        Lang::ZhCn => format!("{count} 条活跃连接 · 累计上传 {upload} · 累计下载 {download}"),
        Lang::En => {
            format!("{count} active connections · uploaded {upload} · downloaded {download}")
        }
    }
}

/// 日志页级别过滤按钮文案。
pub fn fmt_logs_filter(lang: Lang, level_label: &str) -> String {
    match lang {
        Lang::ZhCn => format!("级别：{level_label}"),
        Lang::En => format!("Level: {level_label}"),
    }
}

/// 特权 Helper 状态行。
pub fn fmt_helper_ready(lang: Lang, protocol_version: u32) -> String {
    match lang {
        Lang::ZhCn => format!("就绪（协议版本 {protocol_version}）"),
        Lang::En => format!("Ready (protocol v{protocol_version})"),
    }
}

pub fn fmt_helper_incompatible(lang: Lang, message: &str) -> String {
    match lang {
        Lang::ZhCn => format!("需要修复：{message}"),
        Lang::En => format!("Needs repair: {message}"),
    }
}

/// Mihomo 内核已安装版本行。
pub fn fmt_core_installed(lang: Lang, version: &str) -> String {
    match lang {
        Lang::ZhCn => format!("已安装并校验：{version}"),
        Lang::En => format!("Installed and verified: {version}"),
    }
}

/// 应用更新最新版本行。
pub fn fmt_app_latest(lang: Lang, version: &str, update_available: bool) -> String {
    match (lang, update_available) {
        (Lang::ZhCn, true) => format!("v{version}（有可用更新）"),
        (Lang::ZhCn, false) => format!("v{version}（已是最新）"),
        (Lang::En, true) => format!("v{version} (update available)"),
        (Lang::En, false) => format!("v{version} (up to date)"),
    }
}

/// 应用更新待重启行。
pub fn fmt_pending_restart(lang: Lang, version: &str) -> String {
    match lang {
        Lang::ZhCn => format!("v{version} 已就位，重启后生效"),
        Lang::En => format!("v{version} is ready; takes effect after restart"),
    }
}

/// 诊断/设置导出完成后的路径回显。
pub fn fmt_exported(lang: Lang, path: &str) -> String {
    match lang {
        Lang::ZhCn => format!("已导出：{path}"),
        Lang::En => format!("Exported: {path}"),
    }
}

/// 全局错误条标题。
pub fn fmt_alert_title(lang: Lang, code_debug: &str) -> String {
    format!("{} · {code_debug}", tr(lang, "alert.op_failed"))
}

/// 失败 toast 正文：后端英文 message + 本地化恢复指引。
pub fn fmt_toast_error(lang: Lang, message: &str, hint: Option<&str>) -> String {
    match (lang, hint) {
        (Lang::ZhCn, Some(hint)) => format!("{message}。{hint}。"),
        (Lang::ZhCn, None) => message.to_owned(),
        (Lang::En, Some(hint)) => format!("{message}. {hint}"),
        (Lang::En, None) => message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_have_unique_keys_and_both_languages() {
        let mut keys: Vec<&str> = ENTRIES.iter().map(|(key, _, _)| *key).collect();
        let total = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), total, "i18n key 存在重复");
        for (key, zh, en) in ENTRIES {
            assert!(!zh.is_empty(), "{key} 缺少 zh-CN 文案");
            assert!(!en.is_empty(), "{key} 缺少 en 文案");
            assert_ne!(zh, en, "{key} 两语言文案相同（疑似漏翻）");
        }
    }

    #[test]
    fn tr_looks_up_both_languages() {
        assert_eq!(tr(Lang::ZhCn, "home.title"), "概览");
        assert_eq!(tr(Lang::En, "home.title"), "Overview");
        assert_eq!(tr(Lang::ZhCn, "profiles.import"), "导入配置…");
        assert_eq!(tr(Lang::En, "profiles.import"), "Import Profile…");
    }

    #[test]
    fn lang_maps_settings_language_codes() {
        assert_eq!(Lang::from_code("zh-CN"), Lang::ZhCn);
        assert_eq!(Lang::from_code("en"), Lang::En);
        assert_eq!(Lang::from_code("anything-else"), Lang::En);
    }

    #[test]
    fn fmt_helpers_follow_language_word_order() {
        assert_eq!(
            fmt_reset_scope_title(Lang::ZhCn, "网络"),
            "恢复网络默认值？"
        );
        assert_eq!(
            fmt_reset_scope_title(Lang::En, "Network"),
            "Reset Network to defaults?"
        );
        assert_eq!(
            fmt_connections_summary(Lang::En, 3, "1.0 KB", "2.0 KB"),
            "3 active connections · uploaded 1.0 KB · downloaded 2.0 KB"
        );
        assert_eq!(
            fmt_toast_error(Lang::En, "boom", Some("try again")),
            "boom. try again"
        );
        assert_eq!(fmt_toast_error(Lang::ZhCn, "boom", None), "boom");
    }
}

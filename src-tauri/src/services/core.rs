use crate::utils::{dirs, Command, CommandChild, CommandEvent};
use anyhow::{bail, Result};
use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Core {
    pub core_handler: Arc<RwLock<Option<CommandChild>>>,
}

impl Core {
    pub fn global() -> &'static Core {
        static SERVICE: OnceCell<Core> = OnceCell::new();

        SERVICE.get_or_init(|| Core {
            core_handler: Arc::new(RwLock::new(None)),
        })
    }

    /// 检查sing box配置
    pub async fn check_config(&self) -> Result<()> {
        let config_dir = dirs::app_dir()?;
        let config_dir = dirs::path_to_str(&config_dir)?;
        let core_path = current_core_path()?;
        let output = Command::new_sidecar(core_path)?
            .args(["check", "--disable-color", "-D", config_dir])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("{}", &stderr); // 过滤掉终端颜色值
        }

        Ok(())
    }

    /// 启动核心
    pub async fn run_core(&self) -> Result<()> {
        self.check_config().await?;

        let mut core_handler = self.core_handler.write();

        core_handler.take().map(|ch| {
            let _ = ch.kill();
        });

        let config_dir = dirs::app_dir()?;
        let config_dir = dirs::path_to_str(&config_dir)?;
        let core_path = current_core_path()?;
        let cmd = Command::new_sidecar(&core_path)?;

        #[allow(unused_mut)]
        let (mut rx, cmd_child) = cmd
            .args(["run", "-c", "config.json", "-D", config_dir])
            .spawn()?;

        *core_handler = Some(cmd_child);

        log::info!("run core {core_path}");

        // #[cfg(feature = "stdout-log")]
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Terminated(_) => break,
                    CommandEvent::Error(err) => log::error!("{err}"),
                    CommandEvent::Stdout(line) => {
                        log::info!("{}", String::from_utf8_lossy(&line).trim());
                    }
                    CommandEvent::Stderr(line) => {
                        log::info!("{}", String::from_utf8_lossy(&line).trim());
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }

    // /// 获取所有可执行的文件
    // pub fn list_core() -> Result<Vec<String>> {
    //     let core_dir = dirs::core_dir()?;

    //     let list = std::fs::read_dir(core_dir)?
    //         .filter_map(|e| e.ok())
    //         .filter(|e| e.file_type().map_or(false, |f| f.is_file()))
    //         .map(|e| match e.path().file_stem() {
    //             Some(stem) => stem.to_os_string().into_string().ok(),
    //             None => None,
    //         })
    //         .filter_map(|e| e)
    //         .collect();

    //     Ok(list)
    // }

    // pub fn change_core(&self, name: String) -> Result<()> {
    //     let core_dir = dirs::core_dir()?;

    //     #[cfg(windows)]
    //     let core_path = format!("{name}.exe");
    //     #[cfg(not(windows))]
    //     let core_path = name.clone();
    //     let core_path = core_dir.join(core_path);

    //     if !core_path.exists() {
    //         bail!("core executable file not exists");
    //     }

    //     let sword = Sword::global();
    //     let mut config = sword.config.write();
    //     config.core_name = Some(name);
    //     drop(config);
    //     sword.save_config()?;

    //     self.run_core()?;
    //     Ok(())
    // }
}

/// 获取当前的核心路径
pub fn current_core_path() -> Result<String> {
    let core_name = "sing-box";

    fn use_core_path(name: &str) -> String {
        #[cfg(target_os = "windows")]
        return format!("bin\\{name}");
        #[cfg(not(target_os = "windows"))]
        return format!("bin/{name}");
    }

    Ok(use_core_path(&core_name))
}

//! 数值与时间的小格式化函数，供各页面和状态栏共用。

/// 字节速率：`123 B/s`、`1.2 KB/s`、`3.4 MB/s`。
pub fn rate(bytes_per_second: u64) -> String {
    format!("{}/s", bytes(bytes_per_second))
}

/// 字节量：`123 B`、`1.2 KB`、`3.4 MB`。
pub fn bytes(value: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let value = value as f64;
    if value >= GIB {
        format!("{:.1} GB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KB", value / KIB)
    } else {
        format!("{} B", value as u64)
    }
}

/// 更新间隔秒数的人性化描述。
pub fn interval(seconds: u64) -> String {
    if seconds.is_multiple_of(3600) {
        format!("每 {} 小时", seconds / 3600)
    } else if seconds.is_multiple_of(60) {
        format!("每 {} 分钟", seconds / 60)
    } else {
        format!("每 {seconds} 秒")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_rates_and_bytes() {
        assert_eq!(rate(0), "0 B/s");
        assert_eq!(rate(512), "512 B/s");
        assert_eq!(rate(2048), "2.0 KB/s");
        assert_eq!(bytes(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
    }

    #[test]
    fn formats_update_intervals() {
        assert_eq!(interval(3600), "每 1 小时");
        assert_eq!(interval(300), "每 5 分钟");
        assert_eq!(interval(45), "每 45 秒");
    }
}

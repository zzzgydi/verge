pub struct MapWrapper<'a>(pub &'a serde_yaml::Mapping);

impl<'a> MapWrapper<'a> {
    pub fn has(&self, key: &str) -> bool {
        self.0.get(key).is_some()
    }

    // String
    pub fn get_str(&self, key: &str) -> anyhow::Result<String> {
        self.0
            .get(key)
            .and_then(|s| s.as_str())
            .ok_or_else(|| anyhow::Error::msg(format!("invalid `{}`", key)))
            .map(|s| s.to_owned())
    }

    pub fn or_str(&self, key: &str) -> Option<String> {
        self.0
            .get(key)
            .and_then(|s| s.as_str())
            .map(|s| s.to_owned())
    }

    // i64
    pub fn get_i64(&self, key: &str) -> anyhow::Result<i64> {
        self.0
            .get(key)
            .and_then(|s| s.as_i64())
            .ok_or_else(|| anyhow::Error::msg(format!("invalid `{}`", key)))
    }

    pub fn or_i64(&self, key: &str) -> Option<i64> {
        self.0.get(key).and_then(|s| s.as_i64())
    }

    // u64
    pub fn get_u64(&self, key: &str) -> anyhow::Result<u64> {
        self.0
            .get(key)
            .and_then(|s| s.as_u64())
            .ok_or_else(|| anyhow::Error::msg(format!("invalid `{}`", key)))
    }

    pub fn or_u64(&self, key: &str) -> Option<u64> {
        self.0.get(key).and_then(|s| s.as_u64())
    }

    // bool
    pub fn get_bool(&self, key: &str) -> anyhow::Result<bool> {
        self.0
            .get(key)
            .and_then(|s| s.as_bool())
            .ok_or_else(|| anyhow::Error::msg(format!("invalid `{}`", key)))
    }

    pub fn or_bool(&self, key: &str) -> Option<bool> {
        self.0.get(key).and_then(|s| s.as_bool())
    }

    // vec of string
    pub fn get_vec_str(&self, key: &str) -> anyhow::Result<Vec<String>> {
        self.0
            .get(key)
            .and_then(|s| s.as_sequence())
            .ok_or_else(|| anyhow::Error::msg(format!("invalid `{}`", key)))
            .map(|s| {
                s.iter()
                    .filter_map(|s| s.as_str().map(|s| s.to_owned()))
                    .collect()
            })
    }

    pub fn or_vec_str(&self, key: &str) -> Option<Vec<String>> {
        self.0.get(key).and_then(|s| s.as_sequence()).map(|s| {
            s.iter()
                .filter_map(|s| s.as_str().map(|s| s.to_owned()))
                .collect()
        })
    }
}

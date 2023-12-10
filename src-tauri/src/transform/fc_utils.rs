use super::map_wrapper::MapWrapper;
use std::collections::HashMap;

impl<'a> MapWrapper<'a> {
    pub fn fc_network(&self) -> Option<String> {
        match self.or_bool("udp") {
            Some(true) => None,
            _ => Some("tcp".to_string()),
        }
    }

    pub fn fc_headers(&self) -> Option<HashMap<String, Vec<String>>> {
        self.or_map("headers").map(|h| {
            h.0.iter()
                .map(|(k, v)| {
                    (
                        k.as_str().unwrap_or_default().to_owned(),
                        vec![v.as_str().unwrap_or_default().to_owned()],
                    )
                })
                .collect::<HashMap<String, Vec<String>>>()
        })
    }
}

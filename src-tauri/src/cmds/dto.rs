use serde::{Deserialize, Serialize};

pub type CmdResult<T = ()> = Result<T, String>;

#[macro_export]
macro_rules! wrap_err {
    ($stat: expr) => {
        match $stat {
            Ok(a) => Ok(a),
            Err(err) => {
                log::error!("{}", err.to_string());
                Err(format!("{}", err.to_string()))
            }
        }
    };
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportProfileReq {
    pub url: String,
}

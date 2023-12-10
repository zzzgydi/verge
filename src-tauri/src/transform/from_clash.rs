use std::fmt::{Debug, Display};

pub trait FromClash {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized;
}

pub trait OptionToResult<T> {
    fn okr<M>(self, err: M) -> anyhow::Result<T>
    where
        M: Display + Debug + Send + Sync + 'static;
}

impl<T> OptionToResult<T> for Option<T> {
    fn okr<M>(self, err: M) -> anyhow::Result<T>
    where
        M: Display + Debug + Send + Sync + 'static,
    {
        self.ok_or_else(|| anyhow::Error::msg(err))
    }
}

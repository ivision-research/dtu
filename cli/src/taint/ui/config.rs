use std::fs;

use dtu::Context;

#[derive(Clone, Copy, PartialEq, Eq, Default, PartialOrd, Ord)]
pub enum Detail {
    Minimal = 0,
    #[default]
    Moderate = 1,
    Full = 2,
}

impl Detail {
    pub fn next(&mut self) {
        *self = match *self {
            Self::Minimal => Self::Moderate,
            Self::Moderate => Self::Full,
            Self::Full => Self::Minimal,
        }
    }

    #[allow(unused)]
    pub fn more(&mut self) {
        *self = match *self {
            Self::Minimal => Self::Moderate,
            Self::Moderate | Self::Full => Self::Full,
        }
    }

    #[allow(unused)]
    pub fn less(&mut self) {
        *self = match *self {
            Self::Full => Self::Moderate,
            Self::Moderate | Self::Minimal => Self::Minimal,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "min",
            Self::Moderate => "mod",
            Self::Full => "full",
        }
    }
}

impl<'de> serde::Deserialize<'de> for Detail {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let as_string = <String as serde::Deserialize>::deserialize(deserializer)?;

        Ok(match as_string.as_str() {
            "min" | "mini" | "minimum" => Self::Minimal,
            "mod" | "some" | "moderate" => Self::Moderate,
            "full" | "all" => Self::Full,
            _ => {
                return Err(serde::de::Error::custom(format!(
                    "expected min, mod, full but got: {as_string}"
                )))
            }
        })
    }
}

const fn bool_true() -> bool {
    true
}

#[allow(unused)]
const fn bool_false() -> bool {
    false
}

#[derive(serde::Deserialize, Clone)]
pub struct GraphConfig {
    #[serde(default = "Detail::default")]
    pub detail: Detail,
    #[serde(default = "bool_true")]
    pub show_all: bool,
    #[serde(default = "bool_true")]
    pub show_refs: bool,
    pub hidden: Vec<String>,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            show_all: true,
            show_refs: true,
            hidden: Vec::new(),
            detail: Default::default(),
        }
    }
}

#[derive(serde::Deserialize, Clone)]
pub struct MethodsConfig {
    #[serde(default = "Detail::default")]
    pub detail: Detail,
}

impl Default for MethodsConfig {
    fn default() -> Self {
        Self {
            detail: Default::default(),
        }
    }
}

#[derive(serde::Deserialize, Clone)]
pub struct Config {
    pub graph: GraphConfig,
    pub methods: MethodsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            graph: Default::default(),
            methods: Default::default(),
        }
    }
}

impl Config {
    pub fn load(ctx: &dyn Context) -> dtu::Result<Self> {
        let config = ctx.get_user_config_dir()?.join("taint-ui.toml");

        let slf = if !config.exists() {
            Self::default()
        } else {
            let raw = fs::read_to_string(&config).map_err(|e| {
                dtu::Error::Generic(format!("failed to read file {}: {e}", config.display()))
            })?;
            toml::from_str::<Self>(&raw).map_err(|e| {
                dtu::Error::InvalidConfig(
                    config.display().to_string(),
                    format!("failed to deserialize config: {e}"),
                )
            })?
        };

        Ok(slf)
    }
}

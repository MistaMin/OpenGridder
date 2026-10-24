//! Process isolation sandbox modes (`SANDBOX_NONE`, `SANDBOX_CHAIN`, `SANDBOX_PLUGIN`)
//! porting `Server/Source/Sandbox.hpp` & `Sandbox.cpp`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[repr(i32)]
pub enum SandboxMode {
    #[default]
    None = 0,
    Chain = 1,
    Plugin = 2,
}

impl SandboxMode {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Chain,
            2 => Self::Plugin,
            _ => Self::None,
        }
    }
}

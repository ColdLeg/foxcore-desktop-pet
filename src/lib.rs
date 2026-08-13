//! FoxCore 桌宠插件。
//!
//! 透明无边框置顶窗（eframe/egui）承载一只由活力值驱动动画状态的狐狸，通过
//! `desktop-pet` 适配器接入 FoxCore 消息管线：用户输入 → 核心 agent → 桌宠气泡。
//!
//! 插件名：`foxcore-desktop-pet`
//! 适配器名：`desktop-pet`
//! ABI 版本：1.6（对应 FoxNature v0.2.0 / SDK 0.2.0）

extern crate foxcore_plugin_sdk as abi_stable;

use std::sync::Arc;

use foxcore_plugin_sdk::abi_stable::export_root_module;
use foxcore_plugin_sdk::abi_stable::prefix_type::PrefixTypeTrait;
use foxcore_plugin_sdk::abi_stable::sabi_extern_fn;
use foxcore_plugin_sdk::abi_stable::sabi_trait::TD_Opaque;
use foxcore_plugin_sdk::abi_stable::std_types::{ROption, RResult, RString};
use foxcore_plugin_sdk::{
    AbiError, AbiPluginBox, AbiPlugin_TO, AbiVersion, HostApi, PluginDescriptor, PluginInitInfo,
    PluginMod, PluginModRef, catch_panic,
};

mod channels;
mod config;
mod convert;
mod gui;
mod plugin;
mod theme;
mod vitality;

use plugin::DesktopPetPlugin;

const PLUGIN_NAME: &str = "foxcore-desktop-pet";
const DEFAULT_CONFIG_TOML: &str = include_str!("../default-config.toml");

// ── Root module export ─────────────────────────────────────────────────

#[export_root_module]
#[must_use]
pub fn get_library() -> PluginModRef {
    PluginMod { descriptor, create }.leak_into_prefix()
}

#[sabi_extern_fn]
fn descriptor() -> RResult<PluginDescriptor, AbiError> {
    catch_panic(|| {
        Ok(PluginDescriptor {
            abi_version: AbiVersion::CURRENT,
            name: RString::from(PLUGIN_NAME),
            version: RString::from(env!("CARGO_PKG_VERSION")),
            description: RString::from(
                "FoxCore 桌面桌宠插件：透明无边框置顶窗 + 活力值引擎 + 精力同步",
            ),
            default_config_toml: ROption::RSome(RString::from(DEFAULT_CONFIG_TOML)),
            db_schema: ROption::RNone,
        })
    })
    .into()
}

#[sabi_extern_fn]
fn create(host: HostApi, init: PluginInitInfo) -> RResult<AbiPluginBox, AbiError> {
    catch_panic(|| {
        let config = if init.config_toml.as_str().trim().is_empty() {
            config::DesktopPetConfig::default()
        } else {
            toml::from_str(init.config_toml.as_str()).unwrap_or_default()
        };

        Ok(AbiPlugin_TO::from_value(
            DesktopPetPlugin::new(Arc::new(host), config),
            TD_Opaque,
        ))
    })
    .into()
}

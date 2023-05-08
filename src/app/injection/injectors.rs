#![allow(dead_code)]

use crate::app::engine::{efi_cfg::InjectorConfig, engine_status::InjectionInfo};

pub fn get_base_time(cfg: &InjectorConfig) -> f32 {
    return cfg.on_time - cfg.on_time;
}

pub fn get_battery_correction() -> f32 {
    // esto iria con una tabla
    1.0
}

pub fn get_pressure_correction() -> f32 {
    // por ahora solo disponible en OpenEFI no en uEFI por falta de canales
    // (o uEFI aprovechando las entradas de sensores de dashdash)
    // FIXME: en teoria con llamar a set_injectorFlow ya estaria
    return 0.0;
}

pub fn get_wall_wetting_correction() -> f32 {
    1.0
}

pub fn fuel_mass_to_time(status: &InjectionInfo, fuel: f32) -> f32 {
    // TODO: revisar modo de inyeccion y cilindros
    // monopunto al hacer 4 inyecciones por ciclo seria lo mismo que full secuencial
    // perooo en semi-secuencial al haber dos inyectores, y la mitad de injecciones por ciclo
    // tendria que ser 0.5
    return (fuel / status.fuel_flow_rate * 1000.0f32) / 2.0f32;
}

pub fn set_injector_flow(mut status: InjectionInfo, config: InjectorConfig) {
    // NOTE: como no se revisa la presion del combustible solo se llama una sola vez esto
    let flow = config.fuel_pressure * (config.flow_cc_min * (config.fuel_density / 60.0f32));

    status.fuel_flow_rate = flow;
}

//! Explicit native host diagnostic. Loads only the effect named on the command line.
use og_common::message::PositionInfo;
use og_hosting::{au, vst3, PluginFormatManager};
fn main() {
    if let Err(error) = run() {
        eprintln!("OpenGridder host probe: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let format = args
        .next()
        .ok_or("Usage: host_probe vst3 BUNDLE | au IDENTIFIER")?;
    let id = args.next().ok_or("Missing plugin bundle or identifier")?;
    let desc = match format.as_str() {
        "vst3" => vst3::inspect_vst3_bundle(std::path::Path::new(&id))
            .ok_or("No VST3 factory descriptor")?,
        "au" => au::scan_audiounits()
            .into_iter()
            .find(|p| p.id == id)
            .ok_or("AU not installed")?,
        _ => return Err("Expected vst3 or au".into()),
    };
    let manager = PluginFormatManager::new_all_enabled();
    let mut plugin = manager.create_plugin_instance(&desc, 48000., 1024)?;
    let state = plugin.get_state_information();
    if state.is_empty() {
        return Err("Plugin did not supply state".into());
    }
    let parameters: Vec<_> = plugin
        .parameters()
        .iter()
        .enumerate()
        .map(|(i, p)| serde_json::json!({"index": i,"name":p.name,"value":plugin.get_parameter(i)}))
        .collect();
    let mut midi = Vec::new();
    let mut audio = vec![vec![0.125f32; 64], vec![-0.125; 64]];
    for _ in 0..100 {
        audio[0].fill(0.125);
        audio[1].fill(-0.125);
        plugin.process_block_f32(&mut audio, &mut midi, &PositionInfo::default());
        if audio.iter().flatten().any(|v| !v.is_finite()) {
            return Err("Plugin returned non-finite audio".into());
        }
    }
    let mut second = manager.create_plugin_instance(&desc, 48000., 1024)?;
    second.set_state_information(&state)?;
    for (index, p) in plugin.parameters().iter().enumerate() {
        if (second.get_parameter(index) - plugin.get_parameter(index)).abs() > 1e-5 {
            return Err(format!("State recall differs for {}", p.name));
        }
    }
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"name":desc.name,"id":desc.id,"state_bytes":state.len(),"latency_samples":plugin.latency_samples(),"native_float64":plugin.supports_double_precision(),"output_rms":(audio[0].iter().map(|v|(*v as f64).powi(2)).sum::<f64>()/64.).sqrt(),"parameters":parameters})).unwrap());
    Ok(())
}

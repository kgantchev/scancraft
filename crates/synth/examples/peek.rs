fn main() {
    let args: Vec<String> = std::env::args().collect();
    let curved = args.get(1).is_some_and(|a| a == "curved");
    let count: u32 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(6);
    let spec = scancraft_synth::SceneSpec { count, seed: 7, curved };
    let t = std::time::Instant::now();
    let n = scancraft_synth::scene_io::generate_set(&spec, std::path::Path::new("/tmp/synth-peek")).unwrap();
    eprintln!("{n} scenes in {:.1}s", t.elapsed().as_secs_f64());
}

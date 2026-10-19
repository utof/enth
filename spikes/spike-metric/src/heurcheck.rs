//! `heurcheck`: how good is the heuristic start? Per-parameter median error against the true target parameters.

use clap::Args as ClapArgs;
use spike_metric::distance::median;
use spike_metric::heuristic::heuristic;
use spike_metric::render::{N_PARAMS, PARAM_NAMES, RenderCfg, Wave, render_unit};
use spike_metric::rng::Rng;

#[derive(ClapArgs)]
pub struct Args {
    #[arg(long, default_value_t = 20)]
    targets: usize,
}

pub fn run(args: &Args) {
    let cfg = RenderCfg::default();
    let mut rng = Rng::new(20_261_019); // the same targets as `solve`
    let mut buf = Vec::new();
    let mut errs: Vec<Vec<f64>> = vec![Vec::new(); N_PARAMS - 1];
    let (mut wave_first, mut wave_top2, mut n) = (0, 0, 0);
    while n < args.targets {
        let p: Vec<f64> = (0..N_PARAMS).map(|_| rng.uniform()).collect();
        render_unit(&cfg, &p, &mut buf);
        if (buf.iter().map(|x| x * x).sum::<f32>() / buf.len() as f32).sqrt() <= 1e-3 {
            continue;
        }
        n += 1;
        let st = heuristic(&cfg, &buf);
        let truth = Wave::from_unit(p[0]);
        wave_first += usize::from(st.wave_order[0] == truth);
        wave_top2 += usize::from(st.wave_order[..2].contains(&truth));
        for i in 0..N_PARAMS - 1 {
            errs[i].push((st.cont[i] - p[i + 1]).abs());
        }
    }
    println!(
        "heuristic start on {n} targets: waveform first guess {wave_first}/{n}, in the first two {wave_top2}/{n}"
    );
    println!("median |error| per parameter, normalised to the 0..1 range (random guess ~0.33):");
    for i in 0..N_PARAMS - 1 {
        println!("  {:<20} {:.3}", PARAM_NAMES[i + 1], median(&mut errs[i]));
    }
}

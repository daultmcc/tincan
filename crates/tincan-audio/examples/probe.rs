//! Run the backend headless: `probe [--route] [--output NODE] [--bell F,G,Q] [--seconds N]`.

use tincan_audio::{Backend, Config};
use tincan_core::dsp::{EqParams, SlotParams};
use tincan_core::filter::{Channel, FilterType};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut config = Config { route: false, output: None };
    let mut params = EqParams::default();
    let mut seconds = 5.0;
    let mut n = 0;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--route" => config.route = true,
            "--output" => config.output = args.next(),
            "--seconds" => seconds = args.next().unwrap().parse().unwrap(),
            "--bypass" => params.bypass = true,
            "--preamp" => params.preamp_db = args.next().unwrap().parse().unwrap(),
            "--bell" => {
                let v: Vec<f64> = args.next().unwrap().split(',').map(|x| x.parse().unwrap()).collect();
                params.slots[n] = SlotParams { active: true, kind: FilterType::Bell, channel: Channel::LR, frequency: v[0], gain: v[1], q: v[2] };
                n += 1;
            }
            _ => panic!("unknown argument {a}"),
        }
    }
    let backend = Backend::start(config, &params, |e| println!("event: {e:?}")).expect("start");
    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    println!("clipped: {}", backend.take_clipped());
    backend.shutdown();
    println!("stopped");
}

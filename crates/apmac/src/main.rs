//! Command-line view of what macOS will tell us about connected AirPods, and
//! a way to change the listening mode. Mostly here to check the crate against
//! real hardware without a terminal UI in the way.

use std::process::ExitCode;

use apmac::{NoiseMode, battery, mode};

fn usage() -> ExitCode {
    eprintln!(
        "usage: apmac [status | set <off|nc|transparency|adaptive>]\n\
         \n\
         status  show connected Apple devices, their battery and listening mode\n\
         set     ask macOS to change the listening mode"
    );
    ExitCode::from(2)
}

fn parse_mode(name: &str) -> Option<NoiseMode> {
    match name.to_ascii_lowercase().as_str() {
        "off" => Some(NoiseMode::Off),
        "nc" | "anc" | "noise" | "cancellation" => Some(NoiseMode::NoiseCancellation),
        "transparency" | "trans" => Some(NoiseMode::Transparency),
        "adaptive" => Some(NoiseMode::Adaptive),
        _ => None,
    }
}

fn status() -> ExitCode {
    match battery::connected_airpods() {
        Ok(devices) if devices.is_empty() => println!("no Apple audio device connected"),
        Ok(devices) => {
            for d in devices {
                println!("{} [{}]", d.name, d.address);
                if let Some(fw) = &d.firmware {
                    println!("  firmware   {fw}");
                }
                let battery = [
                    ("left", d.battery_left),
                    ("right", d.battery_right),
                    ("case", d.battery_case),
                    ("battery", d.battery),
                ]
                .into_iter()
                .filter_map(|(label, level)| level.map(|l| format!("{label} {l}%")))
                .collect::<Vec<_>>();
                if !battery.is_empty() {
                    println!("  battery    {}", battery.join(", "));
                }
            }
        }
        Err(e) => {
            eprintln!("apmac: could not read device list: {e}");
            return ExitCode::FAILURE;
        }
    }

    match mode::active() {
        Some(h) => {
            println!("listening mode via CoreAudio on \"{}\"", h.name);
            println!(
                "  current    {}",
                h.mode.map(|m| m.label()).unwrap_or("unknown")
            );
            let supported = h
                .supported
                .iter()
                .map(|m| m.label())
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "  supported  {}",
                if supported.is_empty() {
                    "(none reported)".into()
                } else {
                    supported
                }
            );
        }
        None => println!("no device is reporting a listening mode"),
    }
    ExitCode::SUCCESS
}

fn set(name: &str) -> ExitCode {
    let Some(wanted) = parse_mode(name) else {
        eprintln!("apmac: unknown mode {name:?}");
        return usage();
    };
    let Some(mut pods) = mode::active() else {
        eprintln!(
            "apmac: no device is reporting a listening mode.\n\
             Connect the AirPods and make them the sound output."
        );
        return ExitCode::FAILURE;
    };
    if !pods.supported.is_empty() && !pods.supported.contains(&wanted) {
        eprintln!(
            "apmac: \"{}\" reports support for {} only",
            pods.name,
            pods.supported
                .iter()
                .map(|m| m.label())
                .collect::<Vec<_>>()
                .join(", ")
        );
        return ExitCode::FAILURE;
    }
    // The CLI prints a verdict, so it waits for one rather than trusting the
    // cache's immediate answer.
    match pods.set_mode_confirmed(wanted, std::time::Duration::from_millis(1500)) {
        Ok(()) => {
            println!(
                "{} -> {}",
                pods.name,
                pods.mode.map(|m| m.label()).unwrap_or(wanted.label())
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("apmac: {e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => status(),
        [cmd] if cmd == "status" => status(),
        [cmd, name] if cmd == "set" => set(name),
        _ => usage(),
    }
}

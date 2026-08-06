#[allow(dead_code)]
#[path = "scout_benchmark/enterprise_eval.rs"]
mod enterprise_eval;

fn main() {
    if let Err(error) = run() {
        eprintln!("Scout graph memory benchmark failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let service_count = arguments
        .next()
        .ok_or("usage: scout_graph_memory SERVICES MACHINES lazy|prepared")?
        .parse::<usize>()
        .map_err(|_| "SERVICES must be an integer")?;
    let machine_count = arguments
        .next()
        .ok_or("usage: scout_graph_memory SERVICES MACHINES lazy|prepared")?
        .parse::<usize>()
        .map_err(|_| "MACHINES must be an integer")?;
    let prepare_affected_projection = match arguments
        .next()
        .ok_or("usage: scout_graph_memory SERVICES MACHINES lazy|prepared")?
        .as_str()
    {
        "lazy" => false,
        "prepared" => true,
        _ => return Err("projection mode must be lazy or prepared".into()),
    };
    if arguments.next().is_some() {
        return Err("usage: scout_graph_memory SERVICES MACHINES lazy|prepared".into());
    }
    if service_count == 0 || machine_count == 0 {
        return Err("SERVICES and MACHINES must be nonzero".into());
    }

    let evidence = enterprise_eval::graph_memory_profile(
        service_count,
        machine_count,
        prepare_affected_projection,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
    );
    Ok(())
}

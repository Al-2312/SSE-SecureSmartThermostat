mod database;
mod auth;
mod profiles;
mod sensors;
mod hvac;
mod sim;
mod thermostat;

fn main() {
    dotenvy::dotenv().ok();

    let sim_config = match thermostat::SimConfig::from_env() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Configuration error: {e}");
            std::process::exit(1);
        }
    };

    // The override file is optional, but create the commented-out template so it's there to edit
    match sensors::create_template_if_missing(&sim_config.env_path) {
        Ok(true) => println!("Created {} (edit it to override the weather).", sim_config.env_path.display()),
        Ok(false) => {}
        Err(e) => eprintln!("Couldn't create {}: {e} (using generated weather).", sim_config.env_path.display()),
    }

    let conn = database::init_db().expect("Failed to initialize database");
    profiles::init_profiles(&conn).expect("Failed to set up profiles");
    thermostat::init_thermostat(&conn).expect("Failed to set up thermostat");
    println!("Database initialized successfully!");

    match thermostat::tick_now(&conn, &sim_config) {
        Ok(s) => println!(
            "Simulation running at {}x. Indoor {:.1}°F, outdoor {:.1}°F, HVAC {}.",
            sim_config.speed,
            s.indoor_f,
            s.environment.outdoor_f,
            s.output.as_str()
        ),
        Err(e) => eprintln!("Simulation error: {e:?}"),
    }
}

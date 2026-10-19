mod database;
mod auth;
mod cli;
mod hvac;
mod profiles;
mod sensors;
mod sim;
mod thermostat;
use crate::cli::run_cli;


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
    println!("Database initialized successfully!");

    run_cli(&conn, &sim_config);
  
}        



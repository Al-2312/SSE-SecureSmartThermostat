mod database;
mod auth;
mod profiles;
mod sensors;

fn main() {
    dotenvy::dotenv().ok();

    let conn = database::init_db().expect("Failed to initialize database");
    println!("Database initialized successfully!");

}



use rusqlite::{Connection};
use crate::auth::{self,Role,User};
use crate::database;
use crate::thermostat::{self,SimConfig,Status,ThermostatError};
use crate::profiles::{self,Mode,ProfileError};
use std::io::{self,Write};
use chrono::{self,Utc,Duration};


pub fn print_dashboard(st: &Status) {
    // If active_profile is None, it means the user manually changed the temp/mode
    let profile_label = st.active_profile.as_deref().unwrap_or("Manual (None)");

    println!("\n_________________________________________");
    println!(
        " [Sim Time: {}] | Profile: {} | Mode: {} | Equipment: {}",
        st.sim_time.format("%Y-%m-%d %H:%M"),
        profile_label,
        st.mode.as_str(),
        st.output.as_str()
    );
    println!(
        " INDOOR  -> Temp: {:.1}°F | Target: {}°F | Humidity: {:.0}%",
        st.indoor_f,
        st.target_f,
        st.environment.indoor_humidity_pct
    );
    println!(
        " OUTDOOR -> Temp: {:.1}°F | Humidity: {:.0}% | Precip: {} | CO: {:.1} ppm",
        st.environment.outdoor_f,
        st.environment.outdoor_humidity_pct,
        st.environment.precipitation_label(),
        st.environment.co_ppm
    );

    // If someone puts a bad value in sim/environment.txt, sensor_ok becomes false
    if !st.sensor_ok {
        println!(
            " [!] SENSOR FAULT: {} (Heating & Cooling shut off for safety!)",
            st.sensor_message.as_deref().unwrap_or("Unknown sensor error")
        );
    }

    // Highlight Carbon Monoxide spikes during your demo
    if st.environment.co_ppm > 9.0 {
        println!(
            " [!] WARNING: Elevated Carbon Monoxide level ({:.1} ppm)!",
            st.environment.co_ppm
        );
    }
    println!("_________________________________________");
}

pub fn login(conn: &Connection) -> Option<User> {
    let mut username = String::new();
    print!("Username: ");
    io::stdout().flush().unwrap();
    io::stdin().read_line(&mut username).unwrap();
    let username = username.trim();

    let password = rpassword::prompt_password("Password/PIN: ").unwrap();

    match database::auth_user(conn, username, &password) {
        Ok(user) => Some(user),
        Err(e) => {
            println!("Login failed: {:?}", e);
            None
        }
    }
}

pub fn user_reset(conn: &Connection, user: &User) {
    println!("\n--- Update Profile Credentials ---");
    let final_username =
    loop {
        let mut input = String::new();
        print!("Enter new username (leave blank to keep {}): ", user.username);
        io::stdout().flush().unwrap();
        io::stdin().read_line(&mut input).unwrap();
        let input = input.trim();


        let chosen_name = if input.is_empty() {
            user.username.clone()
        } else {
            input.to_string()
        };

        if chosen_name.eq_ignore_ascii_case("admin") {
            println!("Error: {} is reserved and cannot be used. Please type a different username.", chosen_name);
            continue;
        }

        break chosen_name;
    };
    
    let final_password = loop {
        let pass = rpassword::prompt_password("Enter new complex password: ").unwrap();
        
        match auth::password_check(&pass) {
            Ok(_) => break pass,
            Err(e) => {
                println!("Password does not meet requirements: {:?}", e);
                // The loop automatically restarts, asking them to try again
            }
        }
    };

    let hash = auth::password_hasher(&final_password);

    match database::update_cred(conn, user.id, &final_username, &hash) {
        Ok(_) => println!("Credentials successfully updated!"),
        Err(e) => println!("Failed to update database: {:?}", e),
    }
}

pub fn activate_profile(conn: &Connection, cfg: &SimConfig, user: &User) -> bool  {
    let list = match profiles::list_profiles(conn) {
            Ok(p) => p,
            Err(e) => {
                println!("Failed to list profiles: {:?}", e);
                return false; // Don't log out, just return to menu
            }
        };    

    println!("\n--- Available Profiles ---");
    for p in &list {
        let tag = if p.built_in { "built-in" } else { "custom" };
        println!(
            "{}. {} (Mode: {}, Target: {}°F) [{}]", 
            p.id, p.name, p.mode.as_str(), p.target_f, tag
        );
    }

    print!("Enter Profile ID to activate (or press Enter to cancel): ");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let input = input.trim();

    if input.is_empty() {
        println!("Canceled.");
        return false;
    }

    let Ok(id) = input.parse::<i64>() else {
        println!("Invalid ID. Please enter a number.");
        return false;
    };


    match thermostat::apply_profile(conn, cfg, user, id) {
        Ok(status) => {
            let name = status.active_profile.as_deref().unwrap_or("Unknown");
           println!("\nSuccessfully activated profile: {}", name);
           false
        }
        Err(e) => force_logout(&e),
    }
}

pub fn manual_hvac(conn: &Connection, cfg: &SimConfig, user: &User) -> bool {
    println!("\n-----Manual HVAC Control-----");
    println!("1. Change Mode (Off / Heat / Cool / Auto / Fan)");
    println!("2. Change Target Temperature");
    println!("3. Cancel");
    
    print!("Select an option: ");
    io::stdout().flush().unwrap();
    let mut choice = String::new();
    io::stdin().read_line(&mut choice).unwrap();

    match choice.trim() {
        "1" => {
            print!("Enter mode (Off, Heat, Cool, Auto, Fan): ");
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();

            // Convert string to the Mode enum
            let Some(mode) = Mode::from_str(input.trim()) else {
                println!("Invalid mode. Please type exactly: Off, Heat, Cool, Auto, or Fan.");
                return false;
            };
        match thermostat::set_mode(conn, cfg, user, mode) {
                Ok(_) => {
                    println!("\nHVAC mode updated to {}.", mode.as_str());
                    false
                }
                Err(e) => force_logout(&e),
            }
        }
        "2" => {
            print!("Enter new target temperature ({} - {}°F): ", profiles::MIN_TARGET_F, profiles::MAX_TARGET_F);
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();

            let Ok(target) = input.trim().parse::<i32>() else {
                println!("Invalid temperature. Please enter a whole number.");
                return false;
            };

            match thermostat::set_target(conn, cfg, user, target) {
                Ok(_) => {
                    println!("\nTarget temperature updated to {}°F.", target);
                    false
                }
                Err(e) => force_logout(&e),
            }
        }
        _ => {
            println!("Canceled");
            false
        }
    }
}

fn force_logout(err: &ThermostatError) -> bool {
    match err {
        ThermostatError::AccessExpired => {
            println!("\n Your access window has expired.");
            true
        }
        ThermostatError::AccountInactive => {
            println!("\n Your account has been locked or deactivated.");
            true
        }
        ThermostatError::NotAllowed => {
            println!("Access Denied: Your role does not have permission for this action.");
            false
        }
        ThermostatError::Profile(pe) => {
            println!("Profile Error: {:?}", pe);
            false
        }
        _ => {
            println!("Thermostat Error: {:?}", err);
            false
        }
    }
}


pub fn manage_profiles(conn: &Connection, user: &User) -> bool {
    loop {
        println!("\n--- Manage Thermostat Profiles ---");
        println!("1. Create a New Profile");
        println!("2. Edit an Existing Profile");
        println!("3. Delete a Custom Profile");
        println!("4. Return to Main Menu");

        print!("Select an option: ");
        io::stdout().flush().unwrap();
        let mut choice = String::new();
        io::stdin().read_line(&mut choice).unwrap();

        match choice.trim() {
            "1" => create_profile_cli(conn, user),
            "2" => edit_profile_cli(conn, user),
            "3" => delete_profile_cli(conn, user),
            "4" | "" => return false,
            _ => println!("Invalid option. Please choose 1-4."),
        }
    }
}


fn create_profile_cli(conn: &Connection, user: &User) {
    print!("Enter new profile name (max 32 characters, no special symbols): ");
    io::stdout().flush().unwrap();
    let mut name = String::new();
    io::stdin().read_line(&mut name).unwrap();
    let name = name.trim();

    print!("Enter mode (Off, Heat, Cool, Auto, Fan): ");
    io::stdout().flush().unwrap();
    let mut mode_input = String::new();
    io::stdin().read_line(&mut mode_input).unwrap();
    let Some(mode) = Mode::from_str(mode_input.trim()) else {
        println!("Invalid mode. Creation canceled.");
        return;
    };

    print!("Enter target temperature ({} - {}°F): ", profiles::MIN_TARGET_F, profiles::MAX_TARGET_F);
    io::stdout().flush().unwrap();
    let mut temp_input = String::new();
    io::stdin().read_line(&mut temp_input).unwrap();
    let Ok(target_f) = temp_input.trim().parse::<i32>() else {
        println!("Invalid temperature. Creation canceled.");
        return;
    };

    match profiles::create_profile(conn, user, name, mode, target_f) {
        Ok(_) => println!("Successfully created profile '{}'.", name),
        Err(ProfileError::InvalidName) => println!("Error: Invalid profile name."),
        Err(ProfileError::InvalidTarget) => println!("Error: Temperature out of bounds."),
        Err(ProfileError::DatabaseError(_)) => println!("Error: A profile with that name already exists."),
        Err(e) => println!("Error creating profile: {:?}", e),
    }
}


fn edit_profile_cli(conn: &Connection, user: &User) {
    let list = profiles::list_profiles(conn).unwrap_or_default();
    if list.is_empty() {
        println!("No profiles available.");
        return;
    }

    println!("\n--- Select Profile to Edit ---");
    for p in &list {
        let tag = if p.built_in { "built-in" } else { "custom" };
        println!("{}. {} ({}, {}°F) [{}]", p.id, p.name, p.mode.as_str(), p.target_f, tag);
    }

    print!("Enter the ID of the profile to edit: ");
    io::stdout().flush().unwrap();
    let mut id_input = String::new();
    io::stdin().read_line(&mut id_input).unwrap();
    let Ok(id) = id_input.trim().parse::<i64>() else {
        println!("Invalid ID.");
        return;
    };

    // To keep it simple, we require them to type the name, mode, and temp again
    print!("Enter new name: ");
    io::stdout().flush().unwrap();
    let mut name = String::new();
    io::stdin().read_line(&mut name).unwrap();

    print!("Enter new mode (Off, Heat, Cool, Auto, Fan): ");
    io::stdout().flush().unwrap();
    let mut mode_input = String::new();
    io::stdin().read_line(&mut mode_input).unwrap();
    let Some(mode) = Mode::from_str(mode_input.trim()) else {
        println!("Invalid mode. Edit canceled.");
        return;
    };

    print!("Enter new target temperature: ");
    io::stdout().flush().unwrap();
    let mut temp_input = String::new();
    io::stdin().read_line(&mut temp_input).unwrap();
    let Ok(target_f) = temp_input.trim().parse::<i32>() else {
        println!("Invalid temperature. Edit canceled.");
        return;
    };

    match profiles::update_profile(conn, user, id, name.trim(), mode, target_f) {
        Ok(_) => println!("Successfully updated profile."),
        Err(ProfileError::NotFound) => println!("Error: Profile not found."),
        Err(e) => println!("Error updating profile: {:?}", e),
    }
}

fn delete_profile_cli(conn: &Connection, user: &User) {
    let list = profiles::list_profiles(conn).unwrap_or_default();
    
    println!("\n--- Select Profile to Delete ---");
    for p in &list {
        let tag = if p.built_in { "built-in" } else { "custom" };
        println!("{}. {} [{}]", p.id, p.name, tag);
    }

    print!("Enter the ID of the profile to delete: ");
    io::stdout().flush().unwrap();
    let mut id_input = String::new();
    io::stdin().read_line(&mut id_input).unwrap();
    let Ok(id) = id_input.trim().parse::<i64>() else {
        println!("Invalid ID.");
        return;
    };

    match profiles::delete_profile(conn, user, id) {
        Ok(_) => println!("Successfully deleted profile."),
        Err(ProfileError::BuiltInProtected) => {
            println!("Error: Built-in profiles cannot be deleted. You can only edit them.");
        }
        Err(ProfileError::NotFound) => println!("Error: Profile not found."),
        Err(e) => println!("Error deleting profile: {:?}", e),
    }
}

pub fn energy_report(conn: &Connection, user: &User) -> bool {
    match thermostat::energy_report(conn, user) {
        Ok(report) => {
            println!("\n--- Energy Usage Tracking ---");
            for d in &report.days {
                println!(
                    " Date: {} | Heat: {:.1}m | Cool: {:.1}m | Fan: {:.1}m",
                    d.day,
                    d.heat_secs / 60.0,
                    d.cool_secs / 60.0,
                    d.fan_secs / 60.0
                );
            }
            println!(
                " TOTAL ALL TIME | Heat: {:.1}m | Cool: {:.1}m | Fan: {:.1}m",
                report.total.heat_secs / 60.0,
                report.total.cool_secs / 60.0,
                report.total.fan_secs / 60.0
            );
            false
        }
        Err(e) => force_logout(&e),
    }
}

pub fn create_user_submenu(conn: &Connection) {
    println!("\n--- Create User ---");
    println!("1. Create Guest (Permanent access, PIN required)");
    println!("2. Create Technician (Temporary access, Password required)");
    println!("3. Cancel");
    
    print!("Select an option: ");
    io::stdout().flush().unwrap();
    let mut choice = String::new();
    io::stdin().read_line(&mut choice).unwrap();

    match choice.trim() {
        "1" => create_guest_cli(conn),
        "2" => create_technician_cli(conn),
        _ => println!("Canceled"),
    }
}

fn create_guest_cli(conn: &Connection) {
    print!("Enter new Guest username: ");
    io::stdout().flush().unwrap();
    let mut username = String::new();
    io::stdin().read_line(&mut username).unwrap();
    let username = username.trim();

    // Prevent taking reserved names
    if username.eq_ignore_ascii_case("admin") {
        println!("Error: '{}' is a reserved system name. Choose a different username.", username);
        return;
    }

    let pin = rpassword::prompt_password("Enter a 6 digit PIN for guest: ").unwrap();

    // Validate the PIN rules (Length, Digits, Sequential, Repeating)
        if let Err(e) = auth::pin_check(&pin) {
        println!("Invalid PIN: {:?}", e);
        return; 
    }

    let pin_hash = auth::password_hasher(&pin);

    // Call your database function
    match database::create_user(conn, username, &pin_hash, "Guest", None) {
        Ok(_) => println!("Successfully created Guest: {}", username),
        Err(_) => {
            // Your database function already prints "The username is already taken", 
            // so we don't need to print anything else here.
        }
    }
}

fn create_technician_cli(conn: &Connection) {
    print!("Enter new Technician username: ");
    io::stdout().flush().unwrap();
    let mut username = String::new();
    io::stdin().read_line(&mut username).unwrap();
    let username = username.trim();

    if username.eq_ignore_ascii_case("admin")  {
        println!("Error: '{}' is a reserved system name. Choose a different username.", username);
        return;
    }

    let password = rpassword::prompt_password("Enter complex password: ").unwrap();

    if let Err(e) = auth::password_check(&password) {
        println!("Password does not meet requirements: {:?}", e);
        return;
    }

    print!("How many hours should this technician have access? ");
    io::stdout().flush().unwrap();
    let mut hours_input = String::new();
    io::stdin().read_line(&mut hours_input).unwrap();
    
    let Ok(hours) = hours_input.trim().parse::<i64>() else {
        println!("Invalid input. Please enter a whole number of hours.");
        return;
    };

    if hours <= 0 {
        println!("Error: Hours must be greater than 0.");
        return;
    }

    // Calculate the expiration timestamp
    let expiration_time = Utc::now() + Duration::hours(hours);
    let valid_until = expiration_time.timestamp();

    let password_hash = auth::password_hasher(&password);

    // Call your database function with Some(valid_until)
    match database::create_user(conn, username, &password_hash, "Technician", Some(valid_until)) {
        Ok(_) => {
            println!("Successfully created Technician: {}", username);
            println!("Access expires at: {}", expiration_time.format("%Y-%m-%d %H:%M:%S UTC"));
        }
        Err(_) => {} // Error is handled and printed by the database function
    }
}


pub fn manage_guest_submenu(conn: &Connection) {
    println!("\n--- Manage Guest Accounts ---");
    println!("1. Unlock Guest Account");
    println!("2. Reset Guest PIN");
    println!("3. Revoke Guest Access (Delete Account)");
    println!("4. Cancel");

    print!("Select an option: ");
    io::stdout().flush().unwrap();
    let mut choice = String::new();
    io::stdin().read_line(&mut choice).unwrap();

    match choice.trim() {
        "1" => unlock_guest_cli(conn),
        "2" => reset_guest_pin_cli(conn),
        "3" => revoke_guest_cli(conn),
        "4" | "" => println!("Canceled."),
        _ => println!("Invalid option."),
    }
}

fn unlock_guest_cli(conn: &Connection) {
    print!("Enter the Guest username to unlock: ");
    io::stdout().flush().unwrap();
    let mut username = String::new();
    io::stdin().read_line(&mut username).unwrap();
    let username = username.trim();

    match database::unlock_guest(conn, username) {
        Ok(rows_updated) => {
            if rows_updated > 0 {
                println!("Successfully unlocked guest account: {}", username);
            } else {
                println!("Error: No locked guest account found with that username.");
            }
        }
        Err(e) => println!("Database error: {:?}", e),
    }
}

fn reset_guest_pin_cli(conn: &Connection) {
    print!("Enter the Guest username: ");
    io::stdout().flush().unwrap();
    let mut username = String::new();
    io::stdin().read_line(&mut username).unwrap();
    let username = username.trim();

    let new_pin = rpassword::prompt_password("Enter new 4 or 6 digit PIN: ").unwrap();

    if let Err(e) = auth::pin_check(&new_pin) {
        println!("Invalid PIN: {:?}", e);
        return;
    }

    let pin_hash = auth::password_hasher(&new_pin);

    match database::reset_guest_pin(conn, username, &pin_hash) {
        Ok(rows_updated) => {
            if rows_updated > 0 {
                println!("Successfully updated PIN for guest: {}", username);
            } else {
                println!("Error: No guest account found with that username.");
            }
        }
        Err(e) => println!("Database error: {:?}", e),
    }
}

fn revoke_guest_cli(conn: &Connection) {
    print!("Enter the Guest username to revoke (this will delete the account): ");
    io::stdout().flush().unwrap();
    let mut username = String::new();
    io::stdin().read_line(&mut username).unwrap();
    let username = username.trim();

    // Confirm deletion
    print!("Are you sure you want to revoke '{}'? (y/n): ", username);
    io::stdout().flush().unwrap();
    let mut confirm = String::new();
    io::stdin().read_line(&mut confirm).unwrap();

    if confirm.trim().to_lowercase() != "y" {
        println!("Revocation canceled.");
        return;
    }

    match database::revoke_guest(conn, username) {
        Ok(rows_deleted) => {
            if rows_deleted > 0 {
                println!("Successfully revoked guest access for: {}", username);
            } else {
                println!("Error: No guest account found with that username.");
            }
        }
        Err(e) => println!("Database error: {:?}", e),
    }
}

pub fn manage_tech_submenu(conn: &Connection) {
    println!("\n--- Manage Technician Accounts ---");
    println!("1. Renew Technician Access");
    println!("2. Revoke Technician Access");
    println!("3. Cancel");

    print!("Select an option: ");
    io::stdout().flush().unwrap();
    let mut choice = String::new();
    io::stdin().read_line(&mut choice).unwrap();

    match choice.trim() {
        "1" => renew_technician_cli(conn),
        "2" => revoke_technician_cli(conn),
        "3" | "" => println!("Canceled."),
        _ => println!("Invalid option."),
    }
}

fn renew_technician_cli(conn: &Connection) {
    print!("Enter the Technician username to renew: ");
    io::stdout().flush().unwrap();
    let mut username = String::new();
    io::stdin().read_line(&mut username).unwrap();
    let username = username.trim();

    print!("How many additional hours should they have access? ");
    io::stdout().flush().unwrap();
    let mut hours_input = String::new();
    io::stdin().read_line(&mut hours_input).unwrap();
    
    let Ok(hours) = hours_input.trim().parse::<i64>() else {
        println!("Invalid input. Please enter a whole number of hours.");
        return;
    };

    if hours <= 0 {
        println!("Error: Hours must be greater than 0.");
        return;
    }

    let expiration_time = Utc::now() + Duration::hours(hours);
    let valid_until = expiration_time.timestamp();

    match database::renew_technician(conn, username, valid_until) {
        Ok(rows_updated) => {
            if rows_updated > 0 {
                println!("Successfully renewed Technician: {}", username);
                println!("New access expiration: {}", expiration_time.format("%Y-%m-%d %H:%M:%S UTC"));
            } else {
                println!("Error: No technician account found with that username.");
            }
        }
        Err(e) => println!("Database error: {:?}", e),
    }
}

fn revoke_technician_cli(conn: &Connection) {
    print!("Enter the Technician username to revoke: ");
    io::stdout().flush().unwrap();
    let mut username = String::new();
    io::stdin().read_line(&mut username).unwrap();
    let username = username.trim();

    print!("Are you sure you want to revoke access for '{}'? (y/n): ", username);
    io::stdout().flush().unwrap();
    let mut confirm = String::new();
    io::stdin().read_line(&mut confirm).unwrap();

    if confirm.trim().to_lowercase() != "y" {
        println!("Revocation canceled.");
        return;
    }

    match database::revoke_technician(conn, username) {
        Ok(rows_updated) => {
            if rows_updated > 0 {
                println!("Successfully revoked access for technician: {}", username);
                println!("They have been immediately locked out of the system.");
            } else {
                println!("Error: No technician account found with that username.");
            }
        }
        Err(e) => println!("Database error: {:?}", e),
    }
}


pub fn system_diagnostics(status: &thermostat::Status) {
    println!("\n--- System Diagnostics ---");
    
    // Check HVAC Output state
    println!("Current Equipment State: {}", status.output.as_str());
    
    // Check Sensor Health
    if status.sensor_ok {
        println!("Sensor Status: OK. All environmental sensors reporting valid data.");
    } else {
        println!("Sensor Status: FAULT DETECTED");
        println!("Error Details: {}", status.sensor_message.as_deref().unwrap_or("Unknown Error"));
        println!("System Action: Heating and Cooling have been disabled for safety.");
    }
    
    // Check for hazardous conditions (e.g., CO spikes)
    if status.environment.co_ppm > 9.0 {
        println!("Hazard Warning: Elevated Carbon Monoxide levels detected.");
    } else {
        println!("Hazard Warning: None.");
    }
    
    println!("Diagnostics complete. Press Enter to return.");
    let mut buffer = String::new();
    std::io::stdin().read_line(&mut buffer).unwrap();
}

pub fn run_cli(conn: &Connection, cfg: &SimConfig) {
    loop {
        println!("\n-----SMART THERMOSTAT-----");
        println!("1. Login");
        println!("2. Exit Application");
        println!("Select an option:");

        let mut choice = String::new();

        match io::stdin().read_line(&mut choice) {
            Ok(0) | Err(_) => {
                println!("Exiting application...");
                break;
            }
            Ok(_) => {}
        }

        match choice.trim() {
            "1" => {
                let Some(user) = login(conn) else {
                    continue;
                };

                if user.id == 1 && user.username == "admin" {
                    println!("\x07ALERT: You must change the default admin credentials.");
                    user_reset(conn, &user);
                    println!("Please log in again with your new credentials.");
                    continue;
                }


                match user.role {
                    Role::Homeowner => homeowner_menu(conn, cfg,&user),
                    Role::Technician =>technician_menu(conn,cfg,&user),//technician_menu(),
                    Role::Guest => guest_menu(conn,cfg,&user),//guest_menu(),
                }



                println!("Returning to main menu...");
            }

            "2" => {
                println!("Thank you for using Smart Thermostat!");
                break;
            }

            _ => println!("Invalid option. Please try again."),
        }
    }
}



pub fn homeowner_menu(conn: &Connection, cfg: &SimConfig, user: &User) {
    loop {
        
        let status = match thermostat::tick_now(conn, cfg) {
            Ok(s) => s,
            Err(e) => {
                println!("Thermostat error: {:?}", e);
                break;
            }
        };

        print_dashboard(&status);

        println!("\n-----HOMEOWNER MENU-----");
        println!("1. Activate Thermostat Profile (Home/Away/Sleep/Vacation/Custom)");
        println!("2. Manual HVAC Control (Set Mode / Set Target Temp)");
        println!("3. Manage Thermostat Profiles (Create / Edit / Delete)");
        println!("4. View Energy Report");
        println!("5. Create Guest or Technician");
        println!("6. Manage Guest (Unlock / Reset PIN / Revoke)");
        println!("7. Manage Technician (Renew / Revoke Access)");
        println!("8. Update My Credentials (Username / Password)");
        println!("9. Logout");
        print!("Select an option: ");
        io::stdout().flush().unwrap();

        let mut choice = String::new();

        if std::io::stdin().read_line(&mut choice).is_err() {
            println!("Failed to read input.");
            break;
        }

        let force_logout = match choice.trim() {
            "1" => activate_profile(conn, cfg, user),
            "2" => manual_hvac(conn, cfg, user),
            "3" => manage_profiles(conn, user),
            "4" => energy_report(conn, user),
            "5" => {
                create_user_submenu(conn);
                false
            }
            "6" => {
                manage_guest_submenu(conn);
                false
            }
            "7" => {
                manage_tech_submenu(conn);
                false
            }
            "8" => {
                user_reset(conn, user);
                false
            }
            "9" => {
                println!("Logging out...");
                break;
            }
            _ => {
                println!("Invalid option. Please choose 1-9.");
                false
            }
        };

        if force_logout {
            println!("Session invalidated. Returning to login screen...");
            break;
        }
    }
}

pub fn technician_menu(conn: &Connection, cfg: &SimConfig, user: &User) {
    loop {
        // 1. Advance the simulation and print the live dashboard
        let status = match thermostat::tick_now(conn, cfg) {
            Ok(s) => s,
            Err(e) => {
                println!("Thermostat error: {:?}", e);
                break;
            }
        };

        print_dashboard(&status);

        // 2. Display Technician-specific options
        println!("\n---- TECHNICIAN MENU ----");
        println!("1. Activate Thermostat Profile (Home/Away/Sleep/Vacation/Custom)");
        println!("2. Manual HVAC Control (Set Mode / Set Target Temp)");
        println!("3. View Energy Usage Report");
        println!("4. Run System Diagnostics");
        println!("5. Update My Credentials (Username / Password)");
        println!("6. Logout");
        
        print!("Select an option: ");
        io::stdout().flush().unwrap();

        let mut choice = String::new();
        if io::stdin().read_line(&mut choice).is_err() {
            println!("Failed to read input.");
            break;
        }

        // 3. Route to the correct handler
        let force_logout = match choice.trim() {
            "1" => activate_profile(conn, cfg, user),
            "2" => manual_hvac(conn, cfg, user),
            "3" => energy_report(conn, user),
            "4" => {
                system_diagnostics(&status);
                false // Diagnostics don't interact with the DB, so they won't trigger an expiration kick
            }
            "5" => {
                user_reset(conn, user);
                false
            }
            "6" => {
                println!("Logging out...");
                break;
            }
            _ => {
                println!("Invalid option. Please choose 1-6.");
                false
            }
        };

        // 4. Mid-session termination check
        // If the technician's time window expired while they were looking at a menu,
        // the handler returns true and we kick them out immediately.
        if force_logout {
            println!("Session invalidated. Returning to login screen...");
            break;
        }
    }
}

pub fn guest_menu(conn: &Connection, cfg: &SimConfig, user: &User) {
    loop {
        // 1. Advance the simulation and print the live dashboard
        let status = match thermostat::tick_now(conn, cfg) {
            Ok(s) => s,
            Err(e) => {
                println!("Thermostat error: {:?}", e);
                break;
            }
        };

        print_dashboard(&status);

        // 2. Display Guest-specific options
        println!("\n---- GUEST MENU ----");
        println!("1. Activate Thermostat Profile (Home/Away/Sleep/Vacation/Custom)");
        println!("2. Logout");
        
        print!("Select an option: ");
        io::stdout().flush().unwrap();

        let mut choice = String::new();
        if io::stdin().read_line(&mut choice).is_err() {
            println!("Failed to read input.");
            break;
        }

        // 3. Route to the handler
        let force_logout = match choice.trim() {
            "1" => activate_profile(conn, cfg, user),
            "2" => {
                println!("Logging out...");
                break;
            }
            _ => {
                println!("Invalid option. Please choose 1 or 2.");
                false
            }
        };

        // 4. Mid-session termination check
        // If a Homeowner revoked this guest account while they were sitting at the menu,
        // applying a profile will fail with AccountInactive, triggering an immediate logout.
        if force_logout {
            println!("Session invalidated. Returning to login screen...");
            break;
        }
    }
}
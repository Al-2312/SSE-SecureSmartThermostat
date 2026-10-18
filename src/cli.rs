use std::io;
use crate::auth::Role;
use rusqlite::Connection;
use crate::{auth::User, database}; 
use chrono::Utc; 
use rusqlite::params;
use chrono::Duration; 
use rand_core::TryRng;

pub fn select_role() -> Option<Role> {
    println!("Enter your role (Homeowner/Technician/Guest):");

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok()?;

    Role::str_to_emum(match input.trim().to_lowercase().as_str() {
        "homeowner" | "owner" => "Homeowner",
        "technician" => "Technician",
        "guest" => "Guest",
        _ => return None,
    })
} 
pub fn login(conn: &Connection, role: Role) -> Option<User> {
    let mut username = String::new();

    let password = match role {
    Role::Guest => {
    println!("Enter Guest ID:");
    io::stdin().read_line(&mut username).ok()?;

    rpassword::prompt_password("Enter PIN: ").ok()?
}
    _ => {
        println!("Enter username:");
        io::stdin().read_line(&mut username).ok()?;
        rpassword::prompt_password("Password: ").ok()?
    }
};

    match database::auth_user(conn, username.trim(), &password) {
        Ok(user) if user.role.enum_to_str() == role.enum_to_str() => {
    if access_valid(&user) {
        Some(user)
    } else {
        println!("Access expired or not authorized.");
        None
    }
},
        _ => {
            println!("Invalid credentials or role.");
            None
        }
    }
}

pub fn run_cli(conn: &Connection) {
    let Some(role) = select_role() else {
        println!("Invalid role!");
        return;
    };

    let Some(user) = login(conn, role) else {
        return;
    };

    match user.role {
    Role::Homeowner => {
        println!("1. Create Guest");
        println!("2. Unlock Guest");
        println!("3. Create Technician");
        println!("4. Reset Guest PIN");
        println!("5. Revoke Guest Access");
        println!("6. Revoke Technician Access");
        println!("7. Logout");

        let mut choice = String::new();
        if io::stdin().read_line(&mut choice).is_err() {
            return;
        }

        match choice.trim() {
        "1" => create_guest(conn, &user),
        "2" => unlock_guest(conn, &user),
        "3" => create_technician(conn, &user),
        "4" => reset_guest_pin(conn, &user),
        "5" => revoke_guest_access(conn, &user),
        "6" => revoke_technician_access(conn, &user),
        "7" => println!("Logged out."),
        _ => println!("Invalid option."),
}
    }
    Role::Technician => println!("Technician menu"),
    Role::Guest => println!("Guest menu"),
}
}
pub fn access_valid(user: &User) -> bool {
    match user.valid_until {
        Some(expiry) => Utc::now() < expiry,
        None => !matches!(user.role, Role::Guest | Role::Technician),
    }
}
pub fn valid_pin(pin: &str) -> bool {
    pin.len() == 6
        && pin.bytes().all(|c| c.is_ascii_digit())
        && !["123456", "000000", "111111"].contains(&pin)
} 
pub fn generate_pin() -> Option<String> {
    let mut bytes = [0u8; 4];

    loop {
        rand::rngs::SysRng.try_fill_bytes(&mut bytes).ok()?;

        let number = u32::from_le_bytes(bytes);

        if number < 4_294_000_000 {
            let pin = format!("{:06}", number % 1_000_000);

        if valid_pin(&pin) {
            return Some(pin);
            }
        }
    }
}
pub fn generate_technician_password() -> Option<String> {
    let mut bytes = [0u8; 4];

    rand::rngs::SysRng.try_fill_bytes(&mut bytes).ok()?;

    Some(
        bytes
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect()
    )
}
pub fn create_guest(conn: &Connection, owner: &User) {
    if !matches!(owner.role, Role::Homeowner) || !access_valid(owner) {
    println!("Access denied! Homeowner only.");
    return;
}
    let mut guest_id = String::new();
    println!("Enter Guest ID:");
    io::stdin().read_line(&mut guest_id).unwrap();
    if guest_id.trim().is_empty() {
    println!("Guest ID cannot be empty.");
    return;
}

    let pin = match generate_pin() {
    Some(pin) => pin,
    None => {
        println!("Failed to generate PIN.");
        return;
    }
};

    let mut hours = String::new();
    println!("Access duration (hours):");
    io::stdin().read_line(&mut hours).unwrap();
    let hours: i64 = match hours.trim().parse() {
    Ok(h) if h > 0 && h <= 720 => h,
    _ => {
        println!("Invalid duration! Enter 1–720 hours.");
        return;
    }
};


let expires = Utc::now() + Duration::hours(hours);
let pin_hash = crate::auth::password_hasher(&pin);
let result = conn.execute(
    "INSERT INTO users (username, password_hash, role, valid_until)
     VALUES (?1, ?2, ?3, ?4)",
    params![guest_id.trim(), pin_hash, "Guest", expires.timestamp()],
);
match result {
    Ok(_) => {
        println!("Guest created successfully!");
        println!("Guest ID: {}", guest_id.trim());
        println!("Guest PIN: {}", pin);
        println!("Access expires at: {}", expires);
    }
    Err(e) => {
        println!("Failed to create guest: {}", e);
    }
}
} 
pub fn unlock_guest(conn: &Connection, owner: &User) {
    if !matches!(owner.role, Role::Homeowner) || !access_valid(owner) {
        println!("Access denied! Homeowner only.");
        return;
    }

    let mut guest_id = String::new();

    println!("Enter Guest ID to unlock:");
    if io::stdin().read_line(&mut guest_id).is_err() {
        println!("Failed to read Guest ID.");
        return;
    }

    match conn.execute(
        "UPDATE users
         SET failed_attempts = 0, is_locked = 0
         WHERE username = ?1 AND role = 'Guest'",
        params![guest_id.trim()],
    ) {
        Ok(1) => println!("Guest unlocked successfully!"),
        Ok(0) => println!("Guest not found."),
        Ok(_) => println!("Unexpected number of updated accounts."),
        Err(e) => println!("Failed to unlock guest: {}", e),
    }
}
pub fn create_technician(conn: &Connection, owner: &User) {
    if !matches!(owner.role, Role::Homeowner) || !access_valid(owner) {
        println!("Access denied! Homeowner only.");
        return;
    }

    let mut username = String::new();

    println!("Enter Technician username:");
    if io::stdin().read_line(&mut username).is_err() {
        println!("Failed to read username.");
        return;
    }

    let username = username.trim();

    if username.is_empty() {
        println!("Username cannot be empty.");
        return;
    }

    let mut hours = String::new();

    println!("Access duration (hours):");
    if io::stdin().read_line(&mut hours).is_err() {
        println!("Failed to read duration.");
        return;
    }

    let hours: i64 = match hours.trim().parse() {
        Ok(h) if h > 0 && h <= 720 => h,
        _ => {
            println!("Invalid duration! Enter 1-720 hours.");
            return;
        }
    };

    let password = match generate_technician_password() {
        Some(password) => password,
        None => {
            println!("Failed to generate password.");
            return;
        }
    };

    let password_hash = crate::auth::password_hasher(&password);
    let expires = Utc::now() + Duration::hours(hours);

    let result = conn.execute(
        "INSERT INTO users (username, password_hash, role, valid_until)
         VALUES (?1, ?2, ?3, ?4)",
        params![username, password_hash, "Technician", expires.timestamp()],
    );

    match result {
        Ok(1) => {
            println!("Technician created successfully!");
            println!("Username: {}", username);
            println!("Temporary password: {}", password);
            println!("Access expires at: {}", expires);
        }
        Ok(_) => println!("Technician was not created."),
        Err(e) => println!("Failed to create Technician: {}", e),
    }
}
pub fn reset_guest_pin(conn: &Connection, owner: &User) {
    if !matches!(owner.role, Role::Homeowner) || !access_valid(owner) {
        println!("Access denied! Homeowner only.");
        return;
    }

    let mut guest_id = String::new();

    println!("Enter Guest ID to reset PIN:");

    if io::stdin().read_line(&mut guest_id).is_err() {
        println!("Failed to read Guest ID.");
        return;
    }

    let guest_id = guest_id.trim();

    if guest_id.is_empty() {
        println!("Guest ID cannot be empty.");
        return;
    }

    let new_pin = match generate_pin() {
        Some(pin) => pin,
        None => {
            println!("Failed to generate PIN.");
            return;
        }
    };

    let pin_hash = crate::auth::password_hasher(&new_pin);

    match conn.execute(
        "UPDATE users
         SET password_hash = ?1,
             failed_attempts = 0,
             is_locked = 0
         WHERE username = ?2 AND role = 'Guest'",
        params![pin_hash, guest_id],
    ) {
        Ok(1) => {
            println!("Guest PIN reset successfully!");
            println!("Guest ID: {}", guest_id);
            println!("New PIN: {}", new_pin);
        }
        Ok(0) => println!("Guest not found."),
        Ok(_) => println!("Unexpected number of updated accounts."),
        Err(e) => println!("Failed to reset Guest PIN: {}", e),
    }
} 
pub fn revoke_guest_access(conn: &Connection, owner: &User) {
    if !matches!(owner.role, Role::Homeowner) || !access_valid(owner) {
        println!("Access denied! Homeowner only.");
        return;
    }

    let mut guest_id = String::new();

    println!("Enter Guest ID to revoke access:");

    if io::stdin().read_line(&mut guest_id).is_err() {
        println!("Failed to read Guest ID.");
        return;
    }

    let guest_id = guest_id.trim();

    if guest_id.is_empty() {
        println!("Guest ID cannot be empty.");
        return;
    }

    match conn.execute(
        "UPDATE users
         SET valid_until = ?1
         WHERE username = ?2 AND role = 'Guest'",
        params![Utc::now().timestamp(), guest_id],
    ) {
        Ok(1) => println!("Guest access revoked successfully!"),
        Ok(0) => println!("Guest not found."),
        Ok(_) => println!("Unexpected number of updated accounts."),
        Err(e) => println!("Failed to revoke Guest access: {}", e),
    }
} 
pub fn revoke_technician_access(conn: &Connection, owner: &User) {
    if !matches!(owner.role, Role::Homeowner) || !access_valid(owner) {
        println!("Access denied! Homeowner only.");
        return;
    }

    let mut username = String::new();

    println!("Enter Technician username to revoke access:");

    if io::stdin().read_line(&mut username).is_err() {
        println!("Failed to read username.");
        return;
    }

    let username = username.trim();

    if username.is_empty() {
        println!("Username cannot be empty.");
        return;
    }

    match conn.execute(
        "UPDATE users
         SET valid_until = ?1
         WHERE username = ?2 AND role = 'Technician'",
        params![Utc::now().timestamp(), username],
    ) {
        Ok(1) => println!("Technician access revoked successfully!"),
        Ok(0) => println!("Technician not found."),
        Ok(_) => println!("Unexpected number of updated accounts."),
        Err(e) => println!("Failed to revoke Technician access: {}", e),
    }
}

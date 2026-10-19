use std::{env};


//Here we will create the db if needed 
use rusqlite::{Connection, OptionalExtension, Result, params};
use crate::{auth::{self, Role, User}, profiles, thermostat};
use chrono::{Duration, prelude::*};


#[derive(Debug)]
pub enum AuthError {
    InvalidCredentials,
    AccountLocked, // 
    ExpiredAccount, // Tech
    #[allow(dead_code)]
   DatabaseError(rusqlite::Error)
}

impl From<rusqlite::Error> for AuthError {
    fn from(err: rusqlite::Error) -> Self {
        AuthError::DatabaseError(err)
    }
}



pub fn init_db() -> Result<Connection> {
   
   let db_path= env::var("DB_NAME")
   .expect("Could not find DB_NAME");

   let conn = Connection::open(db_path)?; 
        
    conn.execute(
    "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            username TEXT UNIQUE COLLATE NOCASE NOT NULL,
            password_hash TEXT NOT NULL,
            role TEXT NOT NULL,
            valid_until INTEGER,
            failed_attempts INTEGER DEFAULT 0,
            locked_until INTEGER -- changed to time instead of bool so default is NULL
        );",// might add is lock time 
        [], // no params
    )?;
    
    
    /*// profiles table dont know what to add.
    conn.execute(
    "CREATE TABLE IF NOT EXISTS profiles (
        id INTEGER PRIMARY KEY,
        );",
        [],
    )?;
*/

    let has_users: bool  = conn.query_row(
        "SELECT EXISTS (SELECT 1 from users)",
        [], 
        |row| row.get(0),    
    )?;

    if !has_users {
        let default_password = env::var("DEFAULT_HOMEOWNER_PASSWORD")
        .expect("Could not find DEFAULT_HOMEOWNER_PASSWORD");

        let default_hash = auth::password_hasher(&default_password);
    
        conn.execute(
            "INSERT INTO users (username, password_hash, role) VALUES (?1,?2,?3);", 
            params!["admin", default_hash, "Homeowner"],
        )?;
    } 
    

    profiles::init_profiles(&conn)
        .expect("Failed to initialize profiles table");

    thermostat::init_thermostat(&conn)
        .expect("Failed to initialize thermostat tables");

    Ok(conn)

}


pub fn auth_user(conn: &Connection, username: &str, password: &str) -> Result<User, AuthError> {
   let sql_data = conn.query_row(
    "SELECT id, username, password_hash, role, valid_until, failed_attempts, locked_until 
             FROM users WHERE username = ?1", 
    params![username],
    
    |row| {
        Ok((
        row.get::<_,i64>(0)?, // _ is the same as usize in this case
        row.get::<_,String>(1)?,  
        row.get::<_,String>(2)?,
        row.get::<_,String>(3)?,
        row.get::<_, Option<DateTime<Utc>>>(4)?,
        row.get::<_, i64>(5)?,
        row.get::<_, Option<DateTime<Utc>>>(6)?,    
        ))}
   )
   .optional()?; // "?" error: AuthError` needs to implement `From<rusqlite::Error>`

   let (id, uname, password_hash, role_str, valid_until, failed_attempts, locked_until) = 
   match sql_data {
        Some(data) => data,
        None => return Err(AuthError::InvalidCredentials),
    };

    
    let role= Role::str_to_emum(&role_str).unwrap();

    // check time 
    let current_time = chrono::Utc::now();
    
    if let Some(expiry) = valid_until{
        if current_time > expiry {
            return Err(AuthError::ExpiredAccount);
        }
    }

    if let Some(lock_time) = locked_until{
        if current_time < lock_time {
            return Err(AuthError::AccountLocked);
        }
    }

    let password_valid = auth::password_hash_verify(password, &password_hash);
    let mut lock_10_min=None;
    
    if !password_valid {
        let new_failed = failed_attempts+1;
        let should_lock =  new_failed >= 5;
        if should_lock{
            lock_10_min =  Some(current_time+Duration::minutes(10)) ; // need to convert to seconds i64 using .timestamp
        }
        
        let lock_10_min_timestamp  = lock_10_min.map(|t| t.timestamp());
        
        conn.execute(
            "UPDATE users SET failed_attempts = ?1, locked_until = ?2 WHERE id = ?3",
             params![new_failed, lock_10_min_timestamp, id],
            )?;
        
        if should_lock{
            return Err(AuthError::AccountLocked);
        }
        return Err(AuthError::InvalidCredentials);
    }



    if failed_attempts > 0 {
        conn.execute(
            "UPDATE users SET failed_attempts = 0, locked_until = NULL WHERE id = ?1",
             params![id],
            )?;
    }

     // due 
    Ok(User{
        id: id,
        username: uname,
        role: role,
        valid_until: valid_until,
        failed_attempts: 0,
        //is_locked: lock_10_min, we don't need it   
    })
} 

//here we will ask the user to input the new username and/or password
pub fn update_cred(conn: &Connection, id: i64, new_username: &str, new_password_hash: &str) -> Result<(), rusqlite::Error>{
// we will check to see if the 
    let result= conn.execute(
        "UPDATE users SET username = ?1, password_hash = ?2 WHERE id = ?3",
        params![new_username, new_password_hash, id],
    );
    
    match result{
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure( ffi_error,  Some(text))) if text.contains("UNIQUE constraint failed") =>  {
        //how error of same username Result: UNIQUE constraint failed:
            println!("Error: The username '{}' is already taken.", new_username);
            Err(rusqlite::Error::SqliteFailure(ffi_error, Some(text)))
        }
        Err(e) => Err(e), //anyother error 
    }
}

//ONLY HOMEOWNER can access and will be check in the cli 
pub fn create_user( conn: &Connection, username: &str, password_hash: &str, role: &str, valid_until: Option<i64>) -> Result<(), rusqlite::Error> {
   let result = conn.execute(
        "INSERT INTO users (username, password_hash, role, valid_until) 
         VALUES (?1, ?2, ?3, ?4)",
        params![username, password_hash, role, valid_until],
    );

    match result{
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure( ffi_error,  Some(text))) if text.contains("UNIQUE constraint failed") =>  {
        //how error of same username Result: UNIQUE constraint failed:
            println!("Error: The username '{}' is already taken.", username);
            Err(rusqlite::Error::SqliteFailure(ffi_error, Some(text)))
        }
        Err(e) => Err(e), //anyother error 
    }

}


pub fn renew_technician(conn: &Connection, username: &str, new_valid_until: i64) -> Result<usize, rusqlite::Error> {
    conn.execute(
        "UPDATE users 
         SET valid_until = ?1, failed_attempts = 0, locked_until = NULL 
         WHERE username = ?2 AND role = 'Technician'",
        params![new_valid_until, username],
    )
}

pub fn revoke_technician(conn: &Connection, username: &str) -> Result<usize, rusqlite::Error> {
    // Setting valid_until to NULL causes require_active_account to immediately reject them
    conn.execute(
        "UPDATE users 
         SET valid_until = NULL 
         WHERE username = ?1 AND role = 'Technician'",
        params![username],
    )
}


pub fn unlock_guest(conn: &Connection, username: &str) -> Result<usize, rusqlite::Error> {
    conn.execute(
        "UPDATE users SET failed_attempts = 0, locked_until = NULL 
         WHERE username = ?1 AND role = 'Guest'",
        params![username],
    )
}

pub fn reset_guest_pin(conn: &Connection, username: &str, new_hash: &str) -> Result<usize, rusqlite::Error> {
    conn.execute(
        "UPDATE users SET password_hash = ?1, failed_attempts = 0, locked_until = NULL 
         WHERE username = ?2 AND role = 'Guest'",
        params![new_hash, username],
    )
}

pub fn revoke_guest(conn: &Connection, username: &str) -> Result<usize, rusqlite::Error> {
    conn.execute(
        "DELETE FROM users WHERE username = ?1 AND role = 'Guest'",
        params![username],
    )
}


#[cfg(test)]
mod tests {
use super::*;

    

fn setup_memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("Failed to open in-memory DB");
        conn.execute(
            "CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                username TEXT UNIQUE COLLATE NOCASE NOT NULL,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL,
                valid_until INTEGER,
                failed_attempts INTEGER DEFAULT 0,
                locked_until INTEGER
            );",
            [],
        ).expect("Failed to create users table");
        conn
    }

    #[test]
    fn test_init_db_creates_admin_and_tables() {
        // Set environment variables required by init_db
        // can't think of a better way to test this 
        // due to env::set_var being unsafe in test from parallel 
        unsafe {
            env::set_var("DB_NAME", ":memory:");
            env::set_var("DEFAULT_HOMEOWNER_PASSWORD", "SecurePass123!");
        }
        // This will successfully initialize the users, profiles[cite: 4], and thermostat[cite: 1] tables
        let conn = init_db().expect("Failed to initialize database");

        // Verify the default admin homeowner was created
        let role: String = conn.query_row(
            "SELECT role FROM users WHERE username = 'admin'",
            [],
            |row| row.get(0),
        ).expect("Failed to query admin user");

        assert_eq!(role, "Homeowner");
    }

    #[test]
    fn test_auth_success_and_invalid_credentials() {
        let conn = setup_memory_db();
        let hash = auth::password_hasher("ValidPass123!");
        
        create_user(&conn, "homeowner1", &hash, "Homeowner", None).unwrap();

        // Test Success
        let user = auth_user(&conn, "homeowner1", "ValidPass123!").expect("Auth should succeed");
        assert_eq!(user.username, "homeowner1");
        assert_eq!(user.role, Role::Homeowner);

        // Test Failure
        let err = auth_user(&conn, "homeowner1", "WrongPass!").unwrap_err();
        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    #[test]
    fn test_auth_account_lockout() {
        let conn = setup_memory_db();
        let hash = auth::password_hasher("ValidPass123!");
        
        create_user(&conn, "lock_test", &hash, "Guest", None).unwrap();

        // Fail authentication 4 times (Attempts: 1, 2, 3, 4)
        for _ in 0..4 {
            let err = auth_user(&conn, "lock_test", "BadPass").unwrap_err();
            assert!(matches!(err, AuthError::InvalidCredentials));
        }

        // 5th failure should trigger the lockout
        let err = auth_user(&conn, "lock_test", "BadPass").unwrap_err();
        assert!(matches!(err, AuthError::AccountLocked));

        // Attempting to log in with the CORRECT password while locked must fail
        let err = auth_user(&conn, "lock_test", "ValidPass123!").unwrap_err();
        assert!(matches!(err, AuthError::AccountLocked));

        // Verify database registered the lockout
        let locked_until: Option<i64> = conn.query_row(
            "SELECT locked_until FROM users WHERE username = 'lock_test'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(locked_until.is_some());
    }

    #[test]
    fn test_create_user_unique_constraint() {
        let conn = setup_memory_db();
        
        create_user(&conn, "UniqueUser", "hash1", "Homeowner", None).expect("Should create successfully");

        // Attempt to create the same user. Due to COLLATE NOCASE, varying capitalization should also fail.
        let result = create_user(&conn, "uniqueuser", "hash2", "Guest", None);
        
        assert!(result.is_err());
        if let Err(rusqlite::Error::SqliteFailure(_, Some(msg))) = result {
            assert!(msg.contains("UNIQUE constraint failed"));
        } else {
            panic!("Expected UNIQUE constraint failure");
        }
    }

    #[test]
    fn test_update_credentials() {
        let conn = setup_memory_db();
        create_user(&conn, "OldName", "old_hash", "Homeowner", None).unwrap();

        let id: i64 = conn.query_row("SELECT id FROM users WHERE username = 'OldName'", [], |r| r.get(0)).unwrap();

        // Update name and hash
        update_cred(&conn, id, "NewName", "new_hash").unwrap();

        // Verify change
        let count: i64 = conn.query_row("SELECT count(*) FROM users WHERE username = 'NewName' AND password_hash = 'new_hash'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_technician_lifecycle_and_expiration() {
        let conn = setup_memory_db();
        let hash = auth::password_hasher("TechPass123!");
        
        // Create technician valid for the next 1 hour
        let valid_until = (Utc::now() + Duration::hours(1)).timestamp();
        create_user(&conn, "tech1", &hash, "Technician", Some(valid_until)).unwrap();

        // Revoke access (sets valid_until to NULL)
        let rows = revoke_technician(&conn, "tech1").unwrap();
        assert_eq!(rows, 1);

        let db_valid_until: Option<i64> = conn.query_row("SELECT valid_until FROM users WHERE username = 'tech1'", [], |r| r.get(0)).unwrap();
        assert_eq!(db_valid_until, None);

        // Renew access (sets valid_until to a future time)
        let new_valid_until = (Utc::now() + Duration::hours(2)).timestamp();
        let rows2 = renew_technician(&conn, "tech1", new_valid_until).unwrap();
        assert_eq!(rows2, 1);

        let renewed_valid_until: Option<i64> = conn.query_row("SELECT valid_until FROM users WHERE username = 'tech1'", [], |r| r.get(0)).unwrap();
        assert_eq!(renewed_valid_until, Some(new_valid_until));
    }

    #[test]
    fn test_guest_lifecycle_and_management() {
        let conn = setup_memory_db();
        create_user(&conn, "guest_user", "old_hash", "Guest", None).unwrap();

        // Reset PIN
        let rows = reset_guest_pin(&conn, "guest_user", "new_hash").unwrap();
        assert_eq!(rows, 1);
        
        let hash: String = conn.query_row("SELECT password_hash FROM users WHERE username = 'guest_user'", [], |r| r.get(0)).unwrap();
        assert_eq!(hash, "new_hash");

        // Manually lock the account to test unlock_guest
        conn.execute("UPDATE users SET failed_attempts = 5, locked_until = 9999999999 WHERE username = 'guest_user'", []).unwrap();
        
        let unlock_rows = unlock_guest(&conn, "guest_user").unwrap();
        assert_eq!(unlock_rows, 1);

        let (attempts, locked): (i64, Option<i64>) = conn.query_row(
            "SELECT failed_attempts, locked_until FROM users WHERE username = 'guest_user'", 
            [], 
            |r| Ok((r.get(0)?, r.get(1)?))
        ).unwrap();
        
        assert_eq!(attempts, 0);
        assert_eq!(locked, None);

        // Revoke (Delete) Account
        let del_rows = revoke_guest(&conn, "guest_user").unwrap();
        assert_eq!(del_rows, 1);

        let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE username = 'guest_user')", [], |r| r.get(0)).unwrap();
        assert!(!exists);
    }
}
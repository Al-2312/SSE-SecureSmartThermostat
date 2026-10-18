use std::{default, env};


//Here we will create the db if needed 
use rusqlite::{Connection, OptionalExtension, Result, params};
use crate::auth::{self, Role, User};
use chrono::prelude::*;


#[derive(Debug)]
pub enum AuthError {
    InvalidCredentials,
    AccountLocked, // 
    ExpiredAccount, // Tech
   DatabaseError(rusqlite::Error)
}

impl From<rusqlite::Error> for AuthError {
    fn from(err: rusqlite::Error) -> Self {
        AuthError::DatabaseError(err)
    }
}

#[derive(Debug)]
pub enum CreateAccountError {
    InvalidPassword(auth::PasswordCheckError),
    InvalidPin(auth::PinCheckError),
    Database(rusqlite::Error),
}

impl From<rusqlite::Error> for CreateAccountError {
    fn from(err: rusqlite::Error) -> Self {
        CreateAccountError::Database(err)
    }
}



pub fn init_db() -> Result<Connection> {
   
   let db_path= env::var("DB_NAME")
   .expect("Could not find DB_NAME");

   let conn = Connection::open(db_path)?; 
        
    conn.execute(
    "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            role TEXT NOT NULL,
            valid_until INTEGER,
            failed_attempts INTEGER DEFAULT 0,
            is_locked INTEGER DEFAULT 0
        );",// might add is lock time 
        [], // no params
    )?;

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
    
    Ok(conn)

}

pub fn create_homeowner(conn: &Connection, username: &str, password: &str) -> Result<(), CreateAccountError> {
    auth::password_check(password).map_err(CreateAccountError::InvalidPassword)?;

    let hash = auth::password_hasher(password);
    conn.execute(
        "INSERT INTO users (username, password_hash, role) VALUES (?1, ?2, ?3)",
        params![username, hash, "Homeowner"],
    )?;

    Ok(())
}

pub fn create_guest(conn: &Connection, name: &str, pin: &str) -> Result<(), CreateAccountError> {
    auth::pin_check(pin).map_err(CreateAccountError::InvalidPin)?;

    let hash = auth::password_hasher(pin);
    conn.execute(
        "INSERT INTO users (username, password_hash, role) VALUES (?1, ?2, ?3)",
        params![name, hash, "Guest"],
    )?;

    Ok(())
}

pub fn create_technician(conn: &Connection, username: &str, password: &str) -> Result<(), CreateAccountError> {
    auth::password_check(password).map_err(CreateAccountError::InvalidPassword)?;

    let hash = auth::password_hasher(password);

    // No access window has been opened yet. valid_until is set to an already-expired
    // timestamp so auth_user's expiry check denies login by default, until a
    // window-granting function (homeowner-controlled) sets it to a future time.
    let closed_window = DateTime::<Utc>::from_timestamp(0, 0)
        .expect("unix epoch is always a valid timestamp");

    conn.execute(
        "INSERT INTO users (username, password_hash, role, valid_until) VALUES (?1, ?2, ?3, ?4)",
        params![username, hash, "Technician", closed_window],
    )?;

    Ok(())
}


pub fn auth_user(conn: &Connection, username: &str, password: &str) -> Result<User, AuthError> {
   let sql_data = conn.query_row(
    "SELECT id, username, password_hash, role, valid_until, failed_attempts, is_locked 
             FROM users WHERE username = ?1", 
    params![username],
    
    |row| {
        Ok((
        row.get::<_,u32>(0)?, // _ is the same as usize in this case
        row.get::<_,String>(1)?,  
        row.get::<_,String>(2)?,
        row.get::<_,String>(3)?,
        row.get::<_, Option<DateTime<Utc>>>(4)?,
        row.get::<_, u32>(5)?,
        row.get::<_, bool>(6)?,    
        ))}
   )
   .optional()?; // "?" error: AuthError` needs to implement `From<rusqlite::Error>`

   let (id, uname, password_hash, role_str, valid_until, failed_attempts, is_locked) = 
   match sql_data {
        Some(data) => data,
        None => return Err(AuthError::InvalidCredentials),
    };

    let role= Role::str_to_enum(&role_str).unwrap();

    if is_locked{
        return Err(AuthError::AccountLocked);
    }

    // check time 
    let current_time = chrono::Utc::now();
    
    if let Some(expiry) = valid_until{
        if current_time > expiry {
            return Err(AuthError::ExpiredAccount);
        }
    }

    let password_valid = auth::password_hash_verify(password, &password_hash);

    if !password_valid {
        let new_failed = failed_attempts+1;
        let should_lock = new_failed >= 5;

        conn.execute(
            "UPDATE users SET failed_attempts = ?1, is_locked = ?2 WHERE id = ?3",
             params![new_failed, should_lock, id],
            )?;
        
        if should_lock{
            return Err(AuthError::AccountLocked);
        }
        return Err(AuthError::InvalidCredentials);
    }



    if failed_attempts > 0 {
        conn.execute(
            "UPDATE users SET failed_attempts = 0 WHERE id = ?1",
             params![id],
            )?;
    }

     // due
    Ok(User{
        id,
        username: uname,
        role,
        valid_until,
        failed_attempts: 0,
        is_locked: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                username TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL,
                valid_until INTEGER,
                failed_attempts INTEGER DEFAULT 0,
                is_locked INTEGER DEFAULT 0
            );",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn create_homeowner_then_login() {
        let conn = test_db();
        create_homeowner(&conn, "alice", "CorrectHorse1!").unwrap();

        let user = auth_user(&conn, "alice", "CorrectHorse1!").unwrap();
        assert_eq!(user.username, "alice");
        assert!(matches!(user.role, Role::Homeowner));
    }

    #[test]
    fn create_guest_then_login_with_pin() {
        let conn = test_db();
        create_guest(&conn, "visitor", "482917").unwrap();

        let user = auth_user(&conn, "visitor", "482917").unwrap();
        assert!(matches!(user.role, Role::Guest));
    }

    #[test]
    fn create_guest_rejects_bad_pin() {
        let conn = test_db();
        let err = create_guest(&conn, "visitor", "111111").unwrap_err();
        assert!(matches!(err, CreateAccountError::InvalidPin(_)));
    }

    #[test]
    fn create_technician_cannot_login_until_window_opened() {
        let conn = test_db();
        create_technician(&conn, "bob", "ServiceVisit1!").unwrap();

        let result = auth_user(&conn, "bob", "ServiceVisit1!");
        assert!(matches!(result, Err(AuthError::ExpiredAccount)));
    }

    #[test]
    fn duplicate_name_across_roles_is_rejected() {
        let conn = test_db();
        create_homeowner(&conn, "sam", "CorrectHorse1!").unwrap();

        let err = create_guest(&conn, "sam", "482917").unwrap_err();
        assert!(matches!(err, CreateAccountError::Database(_)));
    }
}

use std::{default, env};


//Here we will create the db if needed 
use rusqlite::{Connection, OptionalExtension, Result, params};
use crate::auth::{self, Role, User};
use chrono::prelude::*;


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

    
    let role= Role::str_to_emum(&role_str).unwrap();

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


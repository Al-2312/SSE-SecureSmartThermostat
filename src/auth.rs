
use argon2::{
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
    Algorithm, Argon2, Params, Version
};

use chrono::prelude::*;
use rusqlite::{Connection, OptionalExtension, Row, params};

pub enum Role{
    Homeowner, 
    Technician,
    Guest, 
}

impl Role {
    // create function that  converts sring of role to enum and vice versa 
    pub fn str_to_emum(role: &str) -> Option<Self>{
        match role{
            "Homeowner"  => Some( Role::Homeowner),
            "Technician" => Some( Role::Technician),
            "Guest"      =>  Some( Role::Guest),
            _            => None,
        }
    }

    pub fn enum_to_str(&self) -> Option<&str>{
        match self{
            Role::Homeowner => Some("Homeowner"),
            Role::Technician => Some("Technician"),
            Role::Guest     =>  Some("Guest"),
            _                => None,      
        }
    }
}
pub struct User{
    pub id: u32,
    pub username: String, 
    pub role: Role,
    pub valid_until: Option<DateTime<Utc>>, // for Technician and it will use chrono time 
    pub failed_attempts: u32,
    pub is_locked: bool,
    // might alter table to add time for lock
}

//before hashing password we must check to see if the password format is correct
pub enum  PasswordCheckError{
    Short,
    Long, //In the case of potnential overflow buffers
    NoUppercase,
    NoNumber,
    NoSpecialChar    
}  


pub fn password_check(password: &str) -> Result< (), PasswordCheckError>{
    if password.len() < 8 {
        return Err(PasswordCheckError::Short);
    }
    if password.len() > 64 {
        return Err(PasswordCheckError::Long);
    } 

    let mut has_uppercase = false;
    let mut has_number = false;
    let mut has_special_char = false;

    for c in password.chars() {
        if c.is_uppercase(){
            has_uppercase = true;
        }
        if c.is_numeric() {
            has_number = true;
        }
        if c.is_ascii_punctuation() {
            has_special_char = true;
        }
    }

    if !has_number{
        return Err(PasswordCheckError::NoNumber);
    }

    if !has_uppercase{
        return Err(PasswordCheckError::NoUppercase);
    }

    if !has_special_char{
        return Err(PasswordCheckError::NoSpecialChar);
    }

    Ok(())
}

pub fn password_hasher(password: &str) -> String {
    
    let argon2 = Argon2::default(); // generates a random salt using OsRng in 0.6.0;
    
    let return_string = argon2.hash_password(password.as_bytes())
    .expect("Failed to hash password")
    .to_string();

    return_string
}

pub fn password_hash_verify(password: &str, password_hashed: &str) -> bool{
    let Ok(hashed)= PasswordHash::new(password_hashed) else {
        return false;
    };

    Argon2::default().verify_password(password.as_bytes(), &hashed).is_ok()
}





#[cfg(test)]
mod tests {
use super::*;

    #[test]
    fn test_hash_and_verify() {
        let password = "TestPassword123!";
        let hash = password_hasher(&password);
        
        //Checking if itt works 
        assert!(password_hash_verify(password, &hash));
        //checking if wrong password is false
        assert!(!password_hash_verify("NotRealPassword", &hash));
    }
}
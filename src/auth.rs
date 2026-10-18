
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
    pub fn str_to_enum(role: &str) -> Option<Self>{
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
#[derive(Debug)]
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

//before hashing a guest PIN we must check its format: 6-12 digits, no repeated digit, no straight run
#[derive(Debug)]
pub enum PinCheckError {
    WrongLength,
    NotAllDigits,
    RepeatedDigit,
    SequentialRun,
}

pub fn pin_check(pin: &str) -> Result<(), PinCheckError> {
    if pin.len() < 6 || pin.len() > 12 {
        return Err(PinCheckError::WrongLength);
    }

    if !pin.chars().all(|c| c.is_ascii_digit()) {
        return Err(PinCheckError::NotAllDigits);
    }

    let digits: Vec<u32> = pin.chars().map(|c| c.to_digit(10).unwrap()).collect();

    if digits.iter().all(|&d| d == digits[0]) {
        return Err(PinCheckError::RepeatedDigit);
    }

    let ascending = digits.windows(2).all(|w| w[1] == w[0] + 1);
    let descending = digits.windows(2).all(|w| w[0] == w[1] + 1);
    if ascending || descending {
        return Err(PinCheckError::SequentialRun);
    }

    Ok(())
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

    #[test]
    fn test_hash_duplicate_check() {
        let password = "TestPassword123!";
        let hash1 = password_hasher(&password);
        let hash2: String = password_hasher(&password); 
        
        //Checking if itt works 
        assert!(password_hash_verify(password, &hash1));
        assert!(password_hash_verify(password, &hash2));
        //checking if wrong password is false
        assert!(!password_hash_verify("NotRealPassword", &hash1));
        assert!(!password_hash_verify("NotRealPassword", &hash2));

        //checking to see if the two hashes are different

        assert_ne!(hash1, hash2);
        println!("Hash1: {}", hash1);
        println!("Hash2: {}", hash2);
    }

    #[test]
    fn pin_check_accepts_valid_pin() {
        assert!(pin_check("482917").is_ok());
    }

    #[test]
    fn pin_check_rejects_bad_length() {
        assert!(matches!(pin_check("1234"), Err(PinCheckError::WrongLength)));
        assert!(matches!(pin_check("1234567890123"), Err(PinCheckError::WrongLength)));
    }

    #[test]
    fn pin_check_rejects_non_digits() {
        assert!(matches!(pin_check("12a456"), Err(PinCheckError::NotAllDigits)));
    }

    #[test]
    fn pin_check_rejects_repeated_digit() {
        assert!(matches!(pin_check("111111"), Err(PinCheckError::RepeatedDigit)));
    }

    #[test]
    fn pin_check_rejects_sequential_run() {
        assert!(matches!(pin_check("123456"), Err(PinCheckError::SequentialRun)));
        assert!(matches!(pin_check("654321"), Err(PinCheckError::SequentialRun)));
    }
}

use argon2::{
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
     Argon2};

use chrono::prelude::*;

#[derive(Debug, PartialEq, Eq, Clone)]
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

     #[allow(dead_code)]
    pub fn enum_to_str(&self) -> Option<&str>{
        match self{
            Role::Homeowner => Some("Homeowner"),
            Role::Technician => Some("Technician"),
            Role::Guest     =>  Some("Guest"),    
        }
    }
}

#[derive(Debug)]
pub struct User{
    pub id: i64,
    pub username: String, 
    pub role: Role,
    #[allow(dead_code)]
    pub valid_until: Option<DateTime<Utc>>, // for Technician and it will use chrono time 
    #[allow(dead_code)]
    pub failed_attempts: i64,
    //pub is_locked: Option<DateTime<Utc>>,
    // might alter table to add time for lock
}

//before hashing password we must check to see if the password format is correct
#[derive(Debug, PartialEq, Eq)]
pub enum  PasswordCheckError{
    Short,
    Long, //In the case of potnential overflow buffers
    NoUppercase,
    NoNumber,
    NoSpecialChar,
    //These next are only for pin
    Sequential,
    Repeating,
    InvaildLength, // could use
    NonDigits   
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

pub fn pin_check(pin: &str) -> Result< (), PasswordCheckError> {
    if pin.len() != 6 {
        return Err(PasswordCheckError::InvaildLength);
    }

    if !pin.chars().all(|c| c.is_ascii_digit()) {
        return Err(PasswordCheckError::NonDigits);
    }

    // it will create a the ascii values of the pin in an array 
    let pin_bytes = pin.as_bytes();

    let first_byte= pin_bytes[0];
    if pin_bytes.iter().all(|&b| b == first_byte) {
        return Err(PasswordCheckError::Repeating);
    }

    let mut is_ascending = true;
    let mut is_descending = true;

    for i in 0..pin_bytes.len() - 1 { 
        // We can just check if the next byte is exactly +1 or -1 away.
        if pin_bytes[i + 1] != pin_bytes[i] + 1 {
            is_ascending = false;
        }
        if pin_bytes[i] != pin_bytes[i + 1] + 1 {
            is_descending = false;
        }
    }

    if is_ascending || is_descending {
        return Err(PasswordCheckError::Sequential);
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


    #[test]
    fn test_hash_unique_salts() {
        // Argon2 should generate a unique salt every time, meaning two hashes 
        // of the exact same password should never equal each other.
        let password = "TestPassword123!";
        let hash1 = password_hasher(password);
        let hash2 = password_hasher(password);
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_role_str_to_enum() {
        assert_eq!(Role::str_to_emum("Homeowner"), Some(Role::Homeowner));
        assert_eq!(Role::str_to_emum("Technician"), Some(Role::Technician));
        assert_eq!(Role::str_to_emum("Guest"), Some(Role::Guest));
        
        // Invalid roles should return None
        assert_eq!(Role::str_to_emum("Admin"), None);
        assert_eq!(Role::str_to_emum("homeowner"), None); // Case sensitive
        assert_eq!(Role::str_to_emum(""), None);
    }

    #[test]
    fn test_role_enum_to_str() {
        assert_eq!(Role::Homeowner.enum_to_str(), Some("Homeowner"));
        assert_eq!(Role::Technician.enum_to_str(), Some("Technician"));
        assert_eq!(Role::Guest.enum_to_str(), Some("Guest"));
    }

    #[test]
    fn test_pin_check(){
        assert_eq!(pin_check("159263"), Ok(()));
        assert_eq!(pin_check("827394"), Ok(()));
        assert_eq!(pin_check("111123"), Ok(()));
        // Testing 5 digits and 7 digits
        assert_eq!(pin_check("12345"), Err(PasswordCheckError::InvaildLength));
        assert_eq!(pin_check("1234567"), Err(PasswordCheckError::InvaildLength));
        //non-digits
        assert_eq!(pin_check("12a456"), Err(PasswordCheckError::NonDigits));
        assert_eq!(pin_check("12!456"), Err(PasswordCheckError::NonDigits));

        assert_eq!(pin_check("111111"), Err(PasswordCheckError::Repeating));
        assert_eq!(pin_check("999999"), Err(PasswordCheckError::Repeating));

        assert_eq!(pin_check("123456"), Err(PasswordCheckError::Sequential));
        assert_eq!(pin_check("345678"), Err(PasswordCheckError::Sequential));

        assert_eq!(pin_check("654321"), Err(PasswordCheckError::Sequential));
        assert_eq!(pin_check("987654"), Err(PasswordCheckError::Sequential));
    }
}
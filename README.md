# Smart Thermostat CLI 

A secure, role-based command-line interface for managing a smart home thermostat. This system features real-time HVAC simulation tracking, environmental monitoring, energy reporting, and strict Role-Based Access Control (RBAC) for Homeowners, Technicians, and Guests.

---

## 1. Installation & Setup

### Prerequisites
* **Rust & Cargo:** Ensure you have the latest stable version of Rust installed (rustup).
* **SQLite3:** Required for the local database backend.

### Environment Configuration
Before running the application, you must configure your environment variables. 
1. Create a file named `.env` in the root directory of the project.
2. Add the following lines to the file:

```text
DB_NAME=thermostat.sqlite3
DEFAULT_HOMEOWNER_PASSWORD=SecurePassword123!
SIM_SPEED = 60
SIM_ENV_FILE = sim/environment.txt
```

## 2. First-Time Setup

When you launch the application for the very first time, the system will automatically build the local SQLite database and provision the default administrative account. 

### Initial Login & Forced Password Reset
To ensure your thermostat is not left vulnerable to default credentials, the system requires an immediate security update upon first boot.

1. At the main menu, select **1. Login**.
2. Enter the default credentials:
   * **Username:** `admin`
   * **Password/PIN:** `SecurePass123!`
3. The system will immediately flag the account and display the following alert:
   > `ALERT: You must change the default admin credentials.`
4. Follow the prompts to enter a new, unique username (the system will reject "admin").
5. Enter a new, complex password that will check if your password is strong .
6. Once completed, the system will log you out. Log back in with your new credentials to access the Homeowner Menu.

---

## 3. Security Requirements

This thermostat implements strict Role-Based Access Control (RBAC) and cryptographic security to protect your home's HVAC system.

* **Password Hashing:** All passwords and PINs are securely hashed using the Argon2 algorithm. Plaintext passwords are never stored in the database.
* **Homeowner & Technician Passwords:** Must be between 8 and 64 characters long and contain at least:
  * One uppercase letter
  * One number
  * One special character (punctuation)
* **Guest PINs:** Must be exactly 6 digits. To prevent easily guessed codes, PINs cannot be purely repeating (e.g., `111111`) and cannot be purely sequential (e.g., `123456` or `987654`).
* **Anti-Brute Force Lockout:** If any user fails to log in 5 times in a row, their account is instantly locked for 10 minutes. Homeowners can manually unlock Guest accounts early if needed.
---

## 4. User Manual & Operations

The system interface adapts based on your role. Below is a guide to performing actions within each permission level.

### The Live Dashboard
Every time you return to your main menu, the system prints the Live Dashboard. This is your primary readout for the house.
* **Profile & Mode:** Shows whether you are running a profile (e.g., "Sleep") or a "Manual" override, and the current HVAC mode (Heat/Cool/Auto/Fan).
* **Equipment:** Shows what the physical HVAC unit is currently doing (e.g., "Heating", "Cooling", or "Idle").
* **Environment:** Displays current indoor and outdoor temperatures, humidity, and weather.
* **Safety Alerts:** If the system detects a hardware fault or hazardous Carbon Monoxide levels (above 9.0 ppm), a warning will appear here, and the HVAC system will shut down automatically for safety.

---

### Homeowner Guide
The Homeowner has absolute control over the system, profiles, and other users.

**How to Activate a Profile:**
1. Select `1` from the Homeowner Menu.
2. The system will list all available built-in and custom profiles.
3. Type the ID number of the profile you want to activate and press Enter. The dashboard will update to reflect the new target temperature.

**How to Manually Override the Temperature:**
1. Select `2` for Manual HVAC Control.
2. Choose to change either the Mode or the Target Temperature.
3. Enter your desired temperature (must be between 30 F and 100 F). This will cancel any active profile and put the system into Manual mode.

**How to Manage Profiles:**
1. Select `3` from the main menu.
2. **Create:** You will be asked for a name, a mode, and a target temperature. Names must be unique.
3. **Edit:** You can change the target temperature or mode of any profile, including built-in ones like "Vacation".
4. **Delete:** You can delete custom profiles. Built-in system profiles cannot be deleted.

**How to Grant Guest Access:**
1. Select `5` to create a user, then select `1` for Guest.
2. Create a username and input a secure 6-digit PIN. Guests have permanent access until you revoke it.

**How to Grant Technician Access:**
1. Select `5` to create a user, then select `2` for Technician.
2. Create a username and a complex password.
3. The system will ask: `How many hours should this technician have access?` 
4. Enter a whole number (e.g., `4`). The system will calculate the exact expiration timestamp. Once that time passes, the technician is automatically locked out.

---

### Technician Guide
Technicians are restricted to diagnostic and testing functions. They cannot manage profiles or users.

**How to Run System Diagnostics:**
1. Select `4` from the Technician Menu.
2. The system will run a status check on the environmental sensors and the physical HVAC equipment.
3. If the dashboard is showing a "SENSOR FAULT", the diagnostic report will provide the specific error details and confirm that heating and cooling have been safely disabled.

**How to View the Energy Report:**
1. Select `3` to pull the Energy Usage Tracking report.
2. The system will display the total runtime (in minutes) for the Heating, Cooling, and Fan systems broken down by day, alongside an all-time historical total.

---

### Guest Guide
Guests have the most restricted access level. They can view the dashboard and activate pre-configured profiles, but cannot alter the system.

**How to Use the Thermostat as a Guest:**
1. Review the Live Dashboard to see the current temperature.
2. Select `1. Activate Thermostat Profile`.
3. Choose a profile from the list provided by the Homeowner (e.g., "Home" or "Sleep") by typing its ID.
4. Select `2` to Logout when finished.
extern crate ganbare;
extern crate diesel;
extern crate handlebars;

#[macro_use] extern crate clap;
extern crate rpassword;
extern crate dotenv;
extern crate rustc_serialize;
#[macro_use]  extern crate lazy_static;

use ganbare::*;
use ganbare::models::User;
use diesel::prelude::*;
use handlebars::Handlebars;
use ganbare::errors::*;
use rustc_serialize::base64::FromBase64;
use std::net::{SocketAddr, ToSocketAddrs};


lazy_static! {

    static ref DATABASE_URL : String = { dotenv::dotenv().ok(); std::env::var("GANBARE_DATABASE_URL")
        .expect("GANBARE_DATABASE_URL must be set (format: postgres://username:password@host/dbname)")};

    static ref RUNTIME_PEPPER : Vec<u8> = { dotenv::dotenv().ok();
        let pepper = std::env::var("GANBARE_RUNTIME_PEPPER")
        .expect("Environmental variable GANBARE_RUNTIME_PEPPER must be set! (format: 256-bit random value encoded as base64)")
        .from_base64().expect("Environmental variable GANBARE_RUNTIME_PEPPER isn't valid Base64!");
        if pepper.len() != 32 { panic!("The value must be 256-bit, that is, 32 bytes long!") }; pepper
    };

    static ref SITE_DOMAIN : String = { dotenv::dotenv().ok(); std::env::var("GANBARE_SITE_DOMAIN")
        .expect("GANBARE_SITE_DOMAIN: Set the site domain! (Without it, the cookies don't work.)") };

    static ref EMAIL_DOMAIN : String = { dotenv::dotenv().ok(); std::env::var("GANBARE_EMAIL_DOMAIN")
        .unwrap_or_else(|_|  std::env::var("GANBARE_SITE_DOMAIN").unwrap_or_else(|_| "".into())) };

    static ref EMAIL_SERVER : SocketAddr = { dotenv::dotenv().ok();
        let binding = std::env::var("GANBARE_EMAIL_SERVER")
        .expect("Specify an outbound email server, like this: mail.yourisp.com:25");
        binding.to_socket_addrs().expect("Format: domain:port").next().expect("Format: domain:port") };

}


pub fn list_users(conn : &PgConnection) -> Result<Vec<User>> {
    use ganbare::schema::users::dsl::*;
 
    users.load::<User>(conn).chain_err(|| "Can't load users")
}


fn main() {
    use clap::*;
    let mut handlebars = Handlebars::new();
    handlebars.register_template_file("email_confirm_email.html", std::path::Path::new("templates/email_confirm_email.html"))
        .expect("Can't register templates/email_confirm_email.html?");

    let matches = App::new("ganba.re user control")
        .setting(AppSettings::SubcommandRequired)
        .version(crate_version!())
        .subcommand(SubCommand::with_name("passwd").about("Set passwords").arg(Arg::with_name("email").required(true)))
        .subcommand(SubCommand::with_name("ls").about("List all users"))
        .subcommand(SubCommand::with_name("rm").about("Remove user").arg(Arg::with_name("email").required(true)))
        .subcommand(SubCommand::with_name("add").about("Add a new user").arg(Arg::with_name("email").required(true)))
        .subcommand(SubCommand::with_name("force_add").about("Add a new user without email confirmation").arg(Arg::with_name("email").required(true)))
        .subcommand(SubCommand::with_name("login").about("Login").arg(Arg::with_name("email").required(true)))
        .get_matches();
    let conn = db_connect(&*DATABASE_URL).unwrap();
    match matches.subcommand() {
        ("passwd", Some(args)) => {
            let email = args.value_of("email").unwrap();
            println!("Setting user {} password.", email);
            println!("Enter a password:");
            let password = match rpassword::read_password() {
                Err(_) => { println!("Error: couldn't read the password from keyboard."); return; },
                Ok(pw) => pw,
            };
            match set_password(&conn, email, &password, &*RUNTIME_PEPPER) {
                Ok(user) => { println!("Success! Password set for user {:?}", user); },
                Err(e) => { println!("Error: {}", e); return; },
            };
        },
        ("ls", Some(_)) => {
            let users = list_users(&conn).unwrap();
            println!("{} users found:", users.len());
            for user in users {
                println!("{:?}", user);
            };
        },
        ("rm", Some(args)) => {
            let email = args.value_of("email").unwrap();
            println!("Removing user with e-mail {}", email);
            match remove_user(&conn, email) {
                Ok(user) => { println!("Success! User removed. Removed user: {:?}", user); },
                Err(e) => { println!("Error: {}", e); return; },
            };
        },
        ("add", Some(args)) => {
            use ganbare::errors::ErrorKind::NoSuchUser;
            let email = args.value_of("email").unwrap();
            match get_user_by_email(&conn, &email) {
                Err(Error(kind, _)) => match kind {
                    NoSuchUser(email) => println!("Adding a user with email {}", email),
                    _ => { println!("Error: {:?}", kind); return; },
                },
                Ok(_) => { println!("Error: User already exists!"); return; },
            }
            let secret = match ganbare::add_pending_email_confirm(&conn, email, &[]) {
                Ok(secret) => secret,
                Err(e) => { println!("Error: {:?}", e); return; }
            };
            match ganbare::email::send_confirmation(email, secret.as_ref(), &*EMAIL_SERVER, &*EMAIL_DOMAIN, &*SITE_DOMAIN, &handlebars) {
                Ok(u) => println!("Sent an email confirmation! {:?}", u),
                Err(err_chain) => for err in err_chain.iter() { println!("Error: {}\nCause: {:?}", err, err.cause ()) },
            }
        },
        ("force_add", Some(args)) => {
            use ganbare::errors::ErrorKind::NoSuchUser;
            let email = args.value_of("email").unwrap();
            match get_user_by_email(&conn, &email) {
                Err(Error(kind, _)) => match kind {
                    NoSuchUser(email) => println!("Adding a user with email {}", email),
                    _ => { println!("Error: {:?}", kind); return; },
                },
                Ok(_) => { println!("Error: User already exists!"); return; },
            }
            println!("Enter a password:");
            let password = match rpassword::read_password() {
                Err(_) => { println!("Error: couldn't read the password from keyboard."); return; },
                Ok(pw) => pw,
            };
            match add_user(&conn, email, &password, &*RUNTIME_PEPPER) {
                Ok(u) => println!("Added user successfully: {:?}", u),
                Err(err_chain) => for err in err_chain.iter() { println!("Error: {}", err) },
            }
        },
        ("login", Some(args)) => {
            let email = args.value_of("email").unwrap();
            println!("Enter a password:");
            let password = match rpassword::read_password() {
                Err(_) => { println!("Error: couldn't read the password from keyboard."); return; },
                Ok(pw) => pw,
            };
            match auth_user(&conn, email, &password, &*RUNTIME_PEPPER) {
                Ok(u) => println!("Logged in successfully: {:?}", u),
                Err(err_chain) => for err in err_chain.iter() { println!("Error: {}", err) },
            }
        },
        _ => {
            unreachable!(); // clap should exit before reaching here if none of the subcommands are entered.
        },
    }
}

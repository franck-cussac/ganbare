#![recursion_limit = "1024"]
#![feature(proc_macro)]
#![feature(custom_derive, custom_attribute, plugin)]
#![plugin(diesel_codegen, dotenv_macros, binary_macros)]

#[macro_use] extern crate diesel;
#[macro_use] extern crate diesel_codegen;
#[macro_use] extern crate error_chain;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate mime;

extern crate dotenv;
extern crate crypto;
extern crate chrono;
extern crate rand;
extern crate rustc_serialize;
extern crate data_encoding;
extern crate pencil;



use diesel::prelude::*;
use dotenv::dotenv;
use std::env;
use std::net::IpAddr;
use std::path::PathBuf;

pub use diesel::pg::PgConnection;

pub mod schema;
pub mod models;
use models::*;
pub mod password;
pub mod errors {

    error_chain! {
        foreign_links {
            ::std::num::ParseIntError, ParseIntError;
            ::std::io::Error, StdIoError;
            ::diesel::result::Error, DieselError;
            ::pencil::PencilError, PencilError;
        }
        errors {
            NoSuchUser(email: String) {
                description("No such user exists")
                display("No user with e-mail address {} exists.", email)
            }
            EmailAddressTooLong {
                description("E-mail address too long")
                display("A valid e-mail address can be 254 characters at maximum.")
            }
            EmailAddressNotValid {
                description("E-mail address not valid")
                display("An e-mail address must contain the character '@'.")
            }
            PasswordTooShort {
                description("Password too short")
                display("A valid password must be at least 8 characters (bytes).")
            }
            PasswordTooLong {
                description("Password too long")
                display("A valid password must be at maximum 1024 characters (bytes).")
            }
            PasswordDoesntMatch {
                description("Password doesn't match")
                display("Password doesn't match.")
            }
            AuthError {
                description("Can't authenticate user")
                display("Username (= e-mail) or password doesn't match.")
            }
            BadSessId {
                description("Malformed session ID!")
                display("Malformed session ID!")
            }
            NoSuchSess {
                description("Session doesn't exist!")
                display("Session doesn't exist!")
            }
            FormParseError {
                description("Can't parse the HTTP form!")
                display("Can't parse the HTTP form!")
            }
            FileNotFound {
                description("Can't find that file!")
                display("Can't find that file!")
            }
        }
    }
}


use errors::*;

pub fn db_connect() -> Result<PgConnection> {
    dotenv().ok();
    let database_url = env::var("GANBARE_DATABASE_URL")
        .chain_err(|| "GANBARE_DATABASE_URL must be set (format: postgres://username:password@host/dbname)")?;
    PgConnection::establish(&database_url)
        .chain_err(|| "Error connecting to database!")
}

pub fn get_user_by_email(conn : &PgConnection, user_email : &str) -> Result<User> {
    use schema::users::dsl::*;
    use diesel::result::Error::NotFound;

    users
        .filter(email.eq(user_email))
        .first(conn)
        .map_err(|e| match e {
                e @ NotFound => e.caused_err(|| ErrorKind::NoSuchUser(user_email.into())),
                e => e.caused_err(|| "Error when trying to retrieve user!"),
        })
}

fn get_user_pass_by_email(conn : &PgConnection, user_email : &str) -> Result<(User, Password)> {
    use schema::users;
    use schema::passwords;
    use diesel::result::Error::NotFound;

    users::table
        .inner_join(passwords::table)
        .filter(users::email.eq(user_email))
        .first(&*conn)
        .map_err(|e| match e {
                e @ NotFound => e.caused_err(|| ErrorKind::NoSuchUser(user_email.into())),
                e => e.caused_err(|| "Error when trying to retrieve user!"),
        })
}

pub fn add_user(conn : &PgConnection, email : &str, password : &str) -> Result<User> {
    use schema::{users, passwords};

    if email.len() > 254 { return Err(ErrorKind::EmailAddressTooLong.into()) };
    if !email.contains("@") { return Err(ErrorKind::EmailAddressNotValid.into()) };

    let pw = password::set_password(password).chain_err(|| "Setting password didn't succeed!")?;

    let new_user = NewUser {
        email : email,
    };

    let user : User = diesel::insert(&new_user)
        .into(users::table)
        .get_result(conn)
        .chain_err(|| "Couldn't create a new user!")?;

    diesel::insert(&pw.into_db(user.id))
        .into(passwords::table)
        .execute(conn)
        .chain_err(|| "Couldn't insert the new password into database!")?;

    Ok(user)
}

pub fn remove_user(conn : &PgConnection, rm_email : &str) -> Result<User> {
    use schema::users::dsl::*;
    use diesel::result::Error::NotFound;

    diesel::delete(users.filter(email.eq(rm_email)))
        .get_result(conn)
        .map_err(|e| match e {
                e @ NotFound => e.caused_err(|| ErrorKind::NoSuchUser(rm_email.into())),
                e => e.caused_err(|| "Couldn't remove the user!"),
        })
}

pub fn auth_user(conn : &PgConnection, email : &str, plaintext_pw : &str) -> Result<User> {
    let (user, hashed_pw_from_db) = get_user_pass_by_email(conn, email)
                                .map_err(|err| {
                                    if let &ErrorKind::NoSuchUser(_) = err.kind() {
                                        return ErrorKind::AuthError.to_err();
                                    };
                                    err
                                })?;
    let _ = password::check_password(plaintext_pw, hashed_pw_from_db.into())
                                .map_err(|err| {
                                    if let &ErrorKind::PasswordDoesntMatch = err.kind() {
                                        return ErrorKind::AuthError.to_err();
                                    };
                                    err
                                })?;
    Ok(user)
}

pub const SESSID_BITS : usize = 128;

/// TODO refactor this function, this is only a temporary helper
pub fn sess_to_hex(sess : &Session) -> String {
    use data_encoding::base16;
    base16::encode(sess.sess_id.as_ref())
}

/// TODO refactor this function, this is only a temporary helper
pub fn sess_to_bin(sessid : &str) -> Result<Vec<u8>> {
    use data_encoding::base16;
    if sessid.len() == SESSID_BITS/4 {
        base16::decode(sessid.as_bytes()).chain_err(|| ErrorKind::BadSessId)
    } else {
        Err(ErrorKind::BadSessId.to_err())
    }
}


pub fn check_session(conn : &PgConnection, session_id : &str) -> Result<(User, Session)> {
    use schema::{users, sessions};
    use diesel::ExpressionMethods;
    use diesel::result::Error::NotFound;

    let (session, user) : (Session, User) = sessions::table
        .inner_join(users::table)
        .filter(sessions::sess_id.eq(sess_to_bin(session_id)?))
        .get_result(conn)
        .map_err(|e| match e {
                e @ NotFound => e.caused_err(|| ErrorKind::NoSuchSess),
                e => e.caused_err(|| "Database error?!"),
        })?;

    Ok((user, session))
} 

pub fn refresh_session(conn : &PgConnection, old_session : &Session, ip : IpAddr) -> Result<Session> {
    use schema::{sessions};
    use diesel::ExpressionMethods;
    use diesel::result::Error::NotFound;

    let new_sessid = fresh_sessid()?;

    let ip_as_bytes = match ip {
        IpAddr::V4(ip) => { ip.octets()[..].to_vec() },
        IpAddr::V6(ip) => { ip.octets()[..].to_vec() },
    };

    let fresh_sess = NewSession {
        sess_id: &new_sessid,
        user_id: old_session.user_id,
        started: old_session.started,
        last_seen: chrono::UTC::now(),
        last_ip: ip_as_bytes,
    };

    let session : Session = diesel::insert(&fresh_sess)
        .into(sessions::table)
        .get_result(conn)
        .map_err(|e| match e {
                e @ NotFound => e.caused_err(|| ErrorKind::NoSuchSess),
                e => e.caused_err(|| "Couldn't update the session."),
        })?;

    // This will delete the user's old sessions IDs, but only after the creation of a new one is underway.
    // In continuous usage, the IDs older than 1 minute will be deleted. But if the user doesn't authenticate,
    // for a while, the newest session IDs will remain until the user returns.
    let num_deleted = diesel::delete(
            sessions::table
                .filter(sessions::user_id.eq(old_session.user_id))
                .filter(sessions::last_seen.lt(chrono::UTC::now()-chrono::Duration::minutes(1)))
        ).execute(&*conn)
        .map_err(|_| "Can't delete old sessions!")?;

    println!("Deleted {:?} sessions", num_deleted);

    Ok(session)
} 

pub fn end_session(conn : &PgConnection, session_id : &str) -> Result<()> {
    use schema::sessions;

    diesel::delete(sessions::table
        .filter(sessions::sess_id.eq(sess_to_bin(session_id).chain_err(|| "Session ID was malformed!")?)))
        .execute(conn)
        .chain_err(|| "Couldn't end the session.")?;
    Ok(())
} 

fn fresh_sessid() -> Result<[u8; SESSID_BITS/8]> {
    use rand::{Rng, OsRng};
    let mut session_id = [0_u8; SESSID_BITS/8];
    OsRng::new().chain_err(|| "Unable to connect to the system random number generator!")?.fill_bytes(&mut session_id);
    Ok(session_id)
}

pub fn start_session(conn : &PgConnection, user : &User, ip : IpAddr) -> Result<Session> {
    use schema::sessions;

    let new_sessid = fresh_sessid()?;

    let ip_as_bytes = match ip {
        IpAddr::V4(ip) => { ip.octets()[..].to_vec() },
        IpAddr::V6(ip) => { ip.octets()[..].to_vec() },
    };

    let new_sess = NewSession {
        sess_id: &new_sessid,
        user_id: user.id,
        started: chrono::UTC::now(),
        last_seen: chrono::UTC::now(),
        last_ip: ip_as_bytes,
    };

    diesel::insert(&new_sess)
        .into(sessions::table)
        .get_result(conn)
        .chain_err(|| "Couldn't start a session!") // TODO if the session id already exists, this is going to fail? (A few-in-a 2^128 change, though...)
}

pub fn add_pending_email_confirm(conn : &PgConnection, email : &str) -> Result<String> {
    use schema::pending_email_confirms;
    let secret = data_encoding::base64url::encode(&fresh_sessid()?[..]);
    {
        let confirm = NewPendingEmailConfirm {
            email: email,
            secret: secret.as_ref(),
        };
        diesel::insert(&confirm)
            .into(pending_email_confirms::table)
            .execute(conn)
            .chain_err(|| "Error :(")?;
    }
    Ok(secret)
}

pub fn check_pending_email_confirm(conn : &PgConnection, secret : &str) -> Result<String> {
    use schema::pending_email_confirms;
    let confirm : PendingEmailConfirm = pending_email_confirms::table
        .filter(pending_email_confirms::secret.eq(secret))
        .first(conn)
        .chain_err(|| "No such secret/email found :(")?;
    Ok(confirm.email)
}

pub fn complete_pending_email_confirm(conn : &PgConnection, password : &str, secret : &str) -> Result<User> {
    use schema::pending_email_confirms;

    let email = check_pending_email_confirm(&*conn, secret)?;
    let user = add_user(&*conn, &email, password)?;

    diesel::delete(pending_email_confirms::table
        .filter(pending_email_confirms::secret.eq(secret)))
        .execute(conn)
        .chain_err(|| "Couldn't delete the pending request.")?;

    Ok(user)
}

#[derive(Debug)]
pub struct Fieldset {
    pub q_variants: Vec<(PathBuf, Option<String>, mime::Mime)>,
    pub answer_audio: Option<(PathBuf, Option<String>, mime::Mime)>,
    pub answer_text: String,
}


pub fn create_quiz(conn : &PgConnection, data: (String, String, String, Vec<Fieldset>)) -> Result<QuizQuestion> {
    use schema::{quiz_questions, question_answers, question_audio, narrators, audio_files};

    println!("Creating quiz!");

    let new_quiz = NewQuizQuestion { skill_id: None, q_name: &data.0, q_explanation: &data.1 };

    let quiz : QuizQuestion = diesel::insert(&new_quiz)
        .into(quiz_questions::table)
        .get_result(&*conn)
        .chain_err(|| "Couldn't create a new question!")?;

    println!("{:?}", &quiz);

    let mut narrator = None;

    fn default_narrator_id(conn: &PgConnection, opt_narrator: &mut Option<Narrator>) -> Result<i32> {
        if let Some(ref narrator) = *opt_narrator {
            Ok(narrator.id)
        } else {

            let new_narrator : Narrator = diesel::insert(&NewNarrator { name: "anonymous" })
                .into(narrators::table)
                .get_result(conn)
                .chain_err(|| "Couldn't create a new narrator!")?;

            println!("{:?}", &new_narrator);
            let narr_id = new_narrator.id;
            *opt_narrator = Some(new_narrator);
            Ok(narr_id)
        }
    }


    for fieldset in &data.3 {
        let a_audio = &fieldset.answer_audio;

        let a_audio_id = if let &Some(ref path_mime) = a_audio {

            let path = path_mime.0.to_str().expect("this is an ascii path!");
            let mime = format!("{}", &path_mime.2);

            let new_a_audio = NewAudioFile {narrators_id: default_narrator_id(&*conn, &mut narrator)?, file_path: path, mime: &mime};
            
            let a_audio : AudioFile = diesel::insert(&new_a_audio)
                .into(audio_files::table)
                .get_result(&*conn)
                .chain_err(|| "Couldn't create a new audio file!")?;
            Some(a_audio.id)

        } else { None };

        println!("{:?}", &a_audio_id);

        let new_answer = NewAnswer { question_id: quiz.id, answer_text: &fieldset.answer_text, audio_files_id: a_audio_id };

        let answer : Answer = diesel::insert(&new_answer)
            .into(question_answers::table)
            .get_result(&*conn)
            .chain_err(|| "Couldn't create a new answer!")?;

        println!("{:?}", &answer);

        for q_audio in &fieldset.q_variants {

            let path = q_audio.0.to_str().expect("this is an ascii path");
            let mime = format!("{}", q_audio.2);
            let new_q_audio = NewAudioFile {narrators_id: default_narrator_id(&*conn, &mut narrator)?, file_path: path, mime: &mime};

            let q_audio : AudioFile = diesel::insert(&new_q_audio)
                .into(audio_files::table)
                .get_result(&*conn)
                .chain_err(|| "Couldn't create a new audio file!")?;

            println!("{:?}", &q_audio);

            let new_q_audio_link = QuestionAudio {id: q_audio.id, question_answers_id: answer.id };

            let q_audio_link : QuestionAudio = diesel::insert(&new_q_audio_link)
                .into(question_audio::table)
                .get_result(&*conn)
                .chain_err(|| "Couldn't create a new question_audio!")?;

            println!("{:?}", &q_audio_link);
        }
        
    }
    Ok(quiz)
}

pub fn get_new_quiz(conn : &PgConnection, user : &User) -> Result<(QuizQuestion, Vec<(Answer, Vec<QuestionAudio>)>)> {
    use schema::{quiz_questions, question_answers};

    // BUG this fails WHEN THERE IS NO QUIZES SAVED, check that case (it's because of .first())

        println!("0");

    let qq : QuizQuestion = quiz_questions::table
        .first(&*conn)
        .chain_err(|| "Can't load quiz!")?;

        println!("1");

    let aas : Vec<Answer> = question_answers::table
        .filter(question_answers::question_id.eq(qq.id))
        .load(&*conn)
        .chain_err(|| "Can't load quiz!")?;

        println!("2");

    let qqs : Vec<Vec<QuestionAudio>> = QuestionAudio
        ::belonging_to(&aas)
        .load(&*conn)
        .chain_err(|| "Can't load quiz!")?
        .grouped_by(&aas);

        println!("3");

    let answers : Vec<(_, Vec<_>)>  = aas.into_iter().zip(qqs).collect();

    Ok((qq, answers))
}

pub fn get_line_file(conn : &PgConnection, line_id : i32) -> Result<(String, mime::Mime)> {
    use schema::audio_files::dsl::*;
    use diesel::result::Error::NotFound;

    let file : AudioFile = audio_files
        .filter(id.eq(line_id))
        .get_result(&*conn)
        .map_err(|e| match e {
                e @ NotFound => e.caused_err(|| ErrorKind::FileNotFound),
                e => e.caused_err(|| "Couldn't get the file!"),
        })?;

    Ok((file.file_path, file.mime.parse().expect("The mimetype from the database should be always valid.")))
}

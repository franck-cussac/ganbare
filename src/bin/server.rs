#![feature(question_mark)]
extern crate ganbare;
extern crate pencil;
extern crate dotenv;
#[macro_use] extern crate log;
extern crate env_logger;
extern crate hyper;
#[macro_use]  extern crate lazy_static;

use dotenv::dotenv;
use std::env;
use ganbare::errors::*;

use std::collections::BTreeMap;
use hyper::header::{SetCookie, CookiePair};
use pencil::{Pencil, Request, PencilResult, redirect, abort};

lazy_static! {
    static ref SITE_DOMAIN : String = { dotenv().ok(); env::var("GANBARE_SITE_DOMAIN")
    .unwrap_or_else(|_| "ENV VAR GANBARE_SITE_DOMAIN NOT SET".into()) };
}

fn hello(request: &mut Request) -> PencilResult {
    let mut context = BTreeMap::new();
    context.insert("title".to_string(), "akusento.ganba.re".to_string());
    return request.app.render_template("hello.html", &context);
}

fn login(request: &mut Request) -> PencilResult {
    let conn = ganbare::db_connect().map_err(|_| abort(500).unwrap_err())?;
    let user;
    {
        let login_form = request.form();
        let email = login_form.get("email").map(String::as_ref).unwrap_or("");
        let plaintext_pw = login_form.get("password").map(String::as_ref).unwrap_or("");
        user = ganbare::auth_user(&conn, email, plaintext_pw)
            .map_err(|e| match e.kind() {
                    &ErrorKind::AuthError => abort(401).unwrap_err(),
                    _ => abort(500).unwrap_err(),
                })?;
    };
    
    let session = ganbare::start_session(&conn, &user, request.request.remote_addr.ip())
        .map_err(|_| abort(500).unwrap_err())?;

    let mut cookie = CookiePair::new("session_id".to_owned(), ganbare::session_id(&session));
    cookie.path = Some("/".to_owned());
    cookie.domain = Some(SITE_DOMAIN.to_owned());

    redirect("/", 303).map(|mut r| { r.set_cookie(SetCookie(vec![cookie])); r })
}

fn main() {
    dotenv().ok();
    let mut app = Pencil::new(".");
    app.register_template("hello.html");
    app.enable_static_file_handling();

    app.set_debug(true);
    app.set_log_level();
    env_logger::init().unwrap();
    debug!("* Running on http://localhost:5000/, serving at {:?}", *SITE_DOMAIN);

    app.get("/", "hello", hello);
    app.post("/login", "login", login);

    let binding = match env::var("GANBARE_SERVER_BINDING") {
        Err(_) => {
            println!("Specify the ip address and port to listen (e.g. 0.0.0.0:80) in envvar GANBARE_SERVER_BINDING!");
            return;
        },
        Ok(ok) => ok,
    };
    println!("Ready to run at {}", binding);
    app.run(binding.as_str());
}
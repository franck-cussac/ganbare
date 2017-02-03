extern crate lettre;
extern crate pencil;
extern crate email as rust_email;

use data_encoding;

use self::lettre::transport::smtp::response::Response as EmailResponse;
use self::lettre::transport::smtp::SmtpTransportBuilder;
use self::lettre::transport::EmailTransport;
use self::lettre::email::EmailBuilder;
use self::pencil::Handlebars;
use std::net::ToSocketAddrs;
use std::collections::BTreeMap;
use rustc_serialize::json::{Json, ToJson};

use schema::pending_email_confirms;
use super::*;

#[derive(RustcEncodable)]
struct EmailData<'a> {
    secret: &'a str,
    site_link: &'a str,
    site_name: &'a str,
}

impl<'a> ToJson for EmailData<'a> {
    fn to_json(&self) -> Json {
        let mut m: BTreeMap<String, Json> = BTreeMap::new();
        m.insert("secret".to_string(), self.secret.to_json());
        m.insert("site_link".to_string(), self.site_link.to_json());
        m.insert("site_name".to_string(), self.site_name.to_json());
        m.to_json()
    }
}

pub fn send_confirmation<SOCK: ToSocketAddrs>(email_addr: &str,
                                              secret: &str,
                                              mail_server: SOCK,
                                              username: &str,
                                              password: &str,
                                              site_name: &str,
                                              site_link: &str,
                                              hb_registry: &Handlebars,
                                              from: (&str, &str))
                                              -> Result<EmailResponse> {

    let data = EmailData {
        secret: secret,
        site_link: site_link,
        site_name: site_name,
    };
    let email = EmailBuilder::new()
        .to(email_addr)
        .from(from)
        .subject(&format!("【{}】Tervetuloa!", site_name))
        .html(hb_registry.render("email_confirm_email.html", &data)
            .chain_err(|| "Handlebars template render error!")?
            .as_ref())
        .build()
        .expect("Building email shouldn't fail.");
    let mut mailer = SmtpTransportBuilder::new(mail_server)
        .chain_err(|| "Couldn't setup the email transport!")?
        .encrypt()
        .credentials(username, password)
        .build();
    mailer.send(email)
        .chain_err(|| "Couldn't send email!")
}

pub fn send_pw_reset_email<SOCK: ToSocketAddrs>(secret: &ResetEmailSecrets,
                                                mail_server: SOCK,
                                                username: &str,
                                                password: &str,
                                                site_name: &str,
                                                site_link: &str,
                                                hb_registry: &Handlebars,
                                                from: (&str, &str))
                                                -> Result<EmailResponse> {

    let data = EmailData {
        secret: &secret.secret,
        site_link: site_link,
        site_name: site_name,
    };
    let email = EmailBuilder::new()
        .to(secret.email.as_str())
        .from(from)
        .subject(&format!("【{}】Salasanan vaihtaminen", site_name))
        .html(hb_registry.render("pw_reset_email.html", &data)
            .chain_err(|| "Handlebars template render error!")?
            .as_ref())
        .build()
        .expect("Building email shouldn't fail.");
    let mut mailer = SmtpTransportBuilder::new(mail_server)
        .chain_err(|| "Couldn't setup the email transport!")?
        .encrypt()
        .credentials(username, password)
        .build();
    mailer.send(email)
        .chain_err(|| "Couldn't send email!")
}

pub fn send_freeform_email<'a, SOCK: ToSocketAddrs, ITER: Iterator<Item = &'a str>>
    (mail_server: SOCK,
     username: &str,
     password: &str,
     from: (&str, &str),
     to: ITER,
     subject: &str,
     body: &str)
     -> Result<()> {

    info!("Going to send email to: {:?}", from);

    let mut mailer = SmtpTransportBuilder::new(mail_server)
        .chain_err(|| "Couldn't setup the email transport!")?
        .encrypt()
        .credentials(username, password)
        .build();

    for to in to {

        let email = EmailBuilder::new()
            .from(from)
            .subject(subject)
            .text(body)
            .to(to)
            .build()
            .expect("Building email shouldn't fail.");

        let result = mailer.send(email)
            .chain_err(|| "Couldn't send!")?;
        info!("Sent freeform emails: {:?}!", result);
    }

    Ok(())
}



pub fn add_pending_email_confirm(conn: &PgConnection,
                                 email: &str,
                                 groups: &[i32])
                                 -> Result<String> {
    let secret = data_encoding::base64url::encode(&session::fresh_token()?[..]);
    {
        let confirm = NewPendingEmailConfirm {
            email: email,
            secret: secret.as_ref(),
            groups: groups,
        };
        diesel::insert(&confirm).into(pending_email_confirms::table)
            .execute(conn)
            .chain_err(|| "Error :(")?;
    }
    Ok(secret)
}

pub fn get_all_pending_email_confirms(conn: &PgConnection) -> Result<Vec<String>> {
    use schema::pending_email_confirms;
    let emails: Vec<String> = pending_email_confirms::table.select(pending_email_confirms::email)
        .get_results(conn)?;

    Ok(emails)
}

pub fn check_pending_email_confirm(conn: &PgConnection,
                                   secret: &str)
                                   -> Result<Option<(String, Vec<i32>)>> {
    let confirm: Option<PendingEmailConfirm> =
        pending_email_confirms::table.filter(pending_email_confirms::secret.eq(secret))
            .first(conn)
            .optional()?;

    Ok(confirm.map(|c| (c.email, c.groups)))
}

pub fn complete_pending_email_confirm(conn: &PgConnection,
                                      password: &str,
                                      secret: &str,
                                      pepper: &[u8])
                                      -> Result<User> {

    let (email, group_ids) = try_or!(check_pending_email_confirm(&*conn, secret)?,
        else return Err(ErrorKind::NoSuchSess.into()));
    let user = user::add_user(&*conn, &email, password, pepper)?;

    for g in group_ids {
        user::join_user_group_by_id(&*conn, user.id, g)?;
    }

    diesel::delete(pending_email_confirms::table
        .filter(pending_email_confirms::secret.eq(secret)))
        .execute(conn)
        .chain_err(|| "Couldn't delete the pending request.")?;

    Ok(user)
}

pub fn clean_old_pendings(conn: &PgConnection, duration: chrono::duration::Duration) -> Result<usize> {
    use schema::pending_email_confirms;
    let deadline = chrono::UTC::now() - duration;
    diesel::delete(pending_email_confirms::table
            .filter(pending_email_confirms::added.lt(deadline)))
        .execute(conn)
        .chain_err(|| "Couldn't delete the old pending requests.")
}

pub fn send_nag_emails<SOCK: ToSocketAddrs>(conn: &PgConnection,
                                            how_old: chrono::Duration,
                                            nag_grace_period: chrono::Duration,
                                            mail_server: SOCK,
                                            username: &str,
                                            password: &str,
                                            site_name: &str,
                                            site_link: &str,
                                            hb_registry: &Handlebars,
                                            from: (&str, &str))
                                            -> Result<()> {

    let slackers = user::get_slackers(conn, how_old)?;

    if slackers.is_empty() {
        return Ok(());
    }

    let mut mailer = SmtpTransportBuilder::new(mail_server)
        .chain_err(|| "Couldn't setup the email transport!")?
        .encrypt()
        .credentials(username, password)
        .build();

    for (user_id, email_addr) in slackers {

        use schema::user_stats;

        let mut stats: UserStats = user_stats::table.filter(user_stats::id.eq(user_id))
            .get_result(conn)?;

        if !user::check_user_group(conn, user_id, "nag_emails")? {
            continue; // We don't send emails to users that don't belong to the "nag_emails" group.
        }

        let last_nag = stats.last_nag_email.unwrap_or_else(|| chrono::date::MIN.and_hms(0, 0, 0));

        if last_nag > chrono::UTC::now() - nag_grace_period {
            continue; // We have sent a nag email recently
        }

        let data = EmailData {
            secret: "",
            site_link: site_link,
            site_name: site_name,
        };
        let email = EmailBuilder::new()
            .to(email_addr.as_str())
            .from(from)
            .subject(&format!("【{}】Minne katosit? (´・ω・`)", site_name))
            .html(hb_registry.render("slacker_heatenings.html", &data) // FIXME
                .chain_err(|| "Handlebars template render error!")?
                .as_ref())
            .build()
            .expect("Building email shouldn't fail.");

        let result = mailer.send(email)
            .chain_err(|| "Couldn't send!")?;

        stats.last_nag_email = Some(chrono::UTC::now());
        let _: UserStats = stats.save_changes(conn)?;

        info!("Sent slacker heatening email to {}: {:?}!",
              email_addr,
              result);
    }

    Ok(())
}

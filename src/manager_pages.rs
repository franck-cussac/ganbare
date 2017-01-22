
use super::*;
use pencil::redirect;
use pencil::abort;
use ganbare::user;
use ganbare::manage;
use std::collections::HashSet;

pub fn fresh_install_form(req: &mut Request) -> PencilResult {
    let conn = ganbare::db::connect(&*DATABASE_URL).err_500()?;
    if ganbare::db::is_installed(&conn).err_500()? {
        return abort(401);
    };
    let context = new_template_context();
    req.app.render_template("fresh_install.html", &context)
}

pub fn fresh_install_post(req: &mut Request) -> PencilResult {
    req.load_form_data();
    let form = req.form().expect("Form data loaded");
    let email = err_400!(form.get("email"), "email missing");
    let new_password = err_400!(form.get("new_password"), "new_password missing");
    let new_password_check = err_400!(form.get("new_password_check"), "new_password_check missing");
    if new_password != new_password_check {
        return Ok(bad_request("passwords don't match"));
    };

    let conn = ganbare::db::connect(&*DATABASE_URL).err_500()?;
    if ganbare::db::is_installed(&conn).err_500()? {
        return abort(401);
    };

    let user = user::add_user(&conn, email, new_password, &*RUNTIME_PEPPER).err_500()?;
    user::join_user_group_by_name(&conn, &user, "admins").err_500()?;
    user::join_user_group_by_name(&conn, &user, "editors").err_500()?;
    user::join_user_group_by_name(&conn, &user, "input_group").err_500()?;
    user::join_user_group_by_name(&conn, &user, "output_group").err_500()?;

    if let Some((_, old_sess)) = get_user(&conn, &*req).err_500()? {
        do_logout(&conn, &old_sess).err_500()?;
    }

    match do_login(&conn,
                   &user.email.expect("The email is known to exist."),
                   new_password,
                   &*req).err_500()? {
        Some((_, sess)) => {
            let mut context = new_template_context();
            context.insert("install_success".into(), "success".into());
            req.app
                .render_template("fresh_install.html", &context)
                .refresh_cookie(&sess)
        }
        None => {
            Err(internal_error(Error::from(ErrMsg("We just added the user, yet we can't login \
                                                   them in. A bug?"
                .to_string()))))
        }
    }
}

pub fn manage(req: &mut Request) -> PencilResult {
    let conn = db_connect().err_500()?;

    let (user, sess) = get_user(&conn, req).err_500()?
        .ok_or_else(|| abort(401).unwrap_err())?; // Unauthorized

    if !user::check_user_group(&conn, user.id, "editors").err_500()? {
        return abort(401);
    }

    let context = new_template_context();

    req.app
        .render_template("manage.html", &context)
        .refresh_cookie(&sess)
}

pub fn add_quiz_form(req: &mut Request) -> PencilResult {

    let (_, _, sess) = auth_user(req, "editors")?;

    let context = new_template_context();

    req.app
        .render_template("add_quiz.html", &context)
        .refresh_cookie(&sess)
}

pub fn add_quiz_post(req: &mut Request) -> PencilResult {

    fn parse_form(req: &mut Request) -> Result<(manage::NewQuestion, Vec<manage::Fieldset>)> {

        req.load_form_data();
        let form = req.form().expect("Form data should be loaded!");
        let files = req.files().expect("Form data should be loaded!");

        let lowest_fieldset = str::parse::<i32>(&parse!(form.get("lowest_fieldset")))?;
        if lowest_fieldset > 10 {
            return Err(ErrorKind::FormParseError.to_err());
        }

        let q_name = parse!(form.get("name"));
        let q_explanation = parse!(form.get("explanation"));
        let question_text = parse!(form.get("question_text"));
        let skill_nugget = parse!(form.get("skill_nugget"));

        let mut fieldsets = Vec::with_capacity(lowest_fieldset as usize);
        for i in 1...lowest_fieldset {

            let q_variations =
                str::parse::<i32>(&parse!(form.get(&format!("choice_{}_q_variations", i))))?;
            if lowest_fieldset > 100 {
                return Err(ErrorKind::FormParseError.to_err());
            }

            let mut q_variants = Vec::with_capacity(q_variations as usize);
            for v in 1...q_variations {
                if let Some(file) = files.get(&format!("choice_{}_q_variant_{}", i, v)) {
                    if file.size.expect("Size should've been parsed at this phase.") == 0 {
                        continue; // Don't save files with size 0;
                    }
                    let mut file = file.clone();
                    file.do_not_delete_on_drop();
                    q_variants.push((file.path.clone(),
                                     file.filename()
                                         .map_err(|_| ErrorKind::FormParseError.to_err())?,
                                     file.content_type()
                                         .ok_or_else(|| ErrorKind::FormParseError.to_err())?));
                }
            }
            if q_variants.is_empty() {
                return Err(Error::from("Can't create a question with 0 audio files for question!"));
            }
            let answer_audio = files.get(&format!("choice_{}_answer_audio", i));
            let answer_audio_path;
            if let Some(path) = answer_audio {
                if path.size.expect("Size should've been parsed at this phase.") == 0 {
                    answer_audio_path = None;
                } else {
                    let mut cloned_path = path.clone();
                    cloned_path.do_not_delete_on_drop();
                    answer_audio_path =
                        Some((cloned_path.path.clone(),
                              cloned_path.filename()
                                  .map_err(|_| ErrorKind::FormParseError.to_err())?,
                              cloned_path.content_type()
                                  .ok_or_else(|| ErrorKind::FormParseError.to_err())?))
                }
            } else {
                answer_audio_path = None;
            };

            let answer_text = parse!(form.get(&format!("choice_{}_answer_text", i)));
            let fields = manage::Fieldset {
                q_variants: q_variants,
                answer_audio: answer_audio_path,
                answer_text: answer_text,
            };
            fieldsets.push(fields);
        }

        Ok((manage::NewQuestion {
                q_name: q_name,
                q_explanation: q_explanation,
                question_text: question_text,
                skill_nugget: skill_nugget,
            },
            fieldsets))
    }

    let (conn, _, sess) = auth_user(req, "editors")?;

    let form = err_400!(parse_form(&mut *req), "Error with parsing form!");
    let result = manage::create_quiz(&conn, form.0, form.1, &*AUDIO_DIR);
    result.map_err(|e| match *e.kind() {
            ErrorKind::FormParseError => abort(400).unwrap_err(),
            _ => abort(500).unwrap_err(),
        })?;

    redirect("/add_quiz", 303).refresh_cookie(&sess)
}

pub fn add_word_form(req: &mut Request) -> PencilResult {
    let (_, _, sess) = auth_user(req, "editors")?;

    let context = new_template_context();

    req.app
        .render_template("add_word.html", &context)
        .refresh_cookie(&sess)
}

pub fn add_word_post(req: &mut Request) -> PencilResult {

    fn parse_form<'a>(req: &'a mut Request) -> Result<manage::NewWordFromStrings<'a>> {

        req.load_form_data();
        let form = req.form().expect("Form data should be loaded!");
        let uploaded_files = req.files().expect("Form data should be loaded!");

        let num_variants = str::parse::<i32>(&parse!(form.get("audio_variations")))?;
        if num_variants > 20 {
            return Err(ErrorKind::FormParseError.to_err());
        }

        let word = parse!(form.get("word"));
        let explanation = parse!(form.get("explanation"));
        let nugget = parse!(form.get("skill_nugget"));

        let mut files = Vec::with_capacity(num_variants as usize);
        for v in 1...num_variants {
            if let Some(file) = uploaded_files.get(&format!("audio_variant_{}", v)) {
                if file.size.expect("Size should've been parsed at this phase.") == 0 {
                    continue; // Don't save files with size 0;
                }
                let mut file = file.clone();
                file.do_not_delete_on_drop();
                files.push((file.path.clone(),
                            file.filename().map_err(|_| ErrorKind::FormParseError.to_err())?,
                            file.content_type()
                                .ok_or_else(|| ErrorKind::FormParseError.to_err())?));
            }
        }

        Ok(manage::NewWordFromStrings {
            word: word,
            explanation: explanation,
            narrator: "",
            nugget: nugget,
            files: files,
            skill_level: 0,
            priority: 0,
        })
    }

    let (conn, _, sess) = auth_user(req, "editors")?;

    let word = parse_form(req).map_err(|_| abort(400).unwrap_err())?;

    manage::create_or_update_word(&conn, word, &*AUDIO_DIR).err_500()?;

    redirect("/add_word", 303).refresh_cookie(&sess)
}

pub fn add_users_form(req: &mut Request) -> PencilResult {

    let (_, _, sess) = auth_user(req, "admins")?;

    let context = new_template_context();
    req.app
        .render_template("add_users.html", &context)
        .refresh_cookie(&sess)
}

pub fn add_users(req: &mut Request) -> PencilResult {
    use ganbare::email;

    let (conn, _, sess) = auth_user(req, "admins")?;

    req.load_form_data();
    let form = req.form().expect("The form data is loaded.");
    let emails = err_400!(form.get("emailList"), "emailList missing?");
    for row in emails.split('\n') {
        let mut fields = row.split_whitespace();
        let email = err_400!(fields.next(), "email field missing?");
        let mut groups = vec![];
        for field in fields {
            groups.push(err_400!(user::get_group(&conn, &field.to_lowercase()).err_500()?,
                                 "No such group?")
                .id);
        }
        let secret = email::add_pending_email_confirm(&conn, email, groups.as_ref()).err_500()?;
        email::send_confirmation(email,
                                 &secret,
                                 &*EMAIL_SERVER,
                                 &*EMAIL_SMTP_USERNAME,
                                 &*EMAIL_SMTP_PASSWORD,
                                 &*SITE_DOMAIN,
                                 &*SITE_LINK,
                                 &**req.app
                                     .handlebars_registry
                                     .read()
                                     .expect("The registry is basically read-only after startup."),
                                 (&*EMAIL_ADDRESS, &*EMAIL_NAME)).err_500()?;
    }

    let context = new_template_context();
    req.app
        .render_template("add_users.html", &context)
        .refresh_cookie(&sess)
}

pub fn users(req: &mut Request) -> PencilResult {
    let (_, _, sess) = auth_user(req, "admins")?;

    let context = new_template_context();

    req.app
        .render_template("users.html", &context)
        .refresh_cookie(&sess)
}

pub fn audio(req: &mut Request) -> PencilResult {
    let (_, _, sess) = auth_user(req, "editors")?;

    let context = new_template_context();

    req.app
        .render_template("audio.html", &context)
        .refresh_cookie(&sess)
}

pub fn send_mail_form(req: &mut Request) -> PencilResult {
    let (_, _, sess) = auth_user(req, "editors")?;

    let sent = req.form_mut().take("sent");

    let mut context = new_template_context();

    if let Some(sent) = sent {
        context.insert("sent".into(), sent);
    }

    req.app
        .render_template("send_mail.html", &context)
        .refresh_cookie(&sess)
}

pub fn send_mail_post(req: &mut Request) -> PencilResult {
    let (conn, _, sess) = auth_user(req, "editors")?;

    let group_pending = req.form_mut().take("group_pending");
    let group = req.form_mut().take_all("group[]").unwrap_or_else(|| vec![]);
    if group_pending.is_none() && group.is_empty() {
        return Ok(bad_request("group is missing!"));
    }
    let group =
        err_400!(group.into_iter().map(|id| str::parse::<i32>(&id)).collect::<Vec<_>>().flip(),
                 "group invalid");
    let subject = err_400!(req.form_mut().take("subject"), "subject missing");
    let body = err_400!(req.form_mut().take("body"), "body missing");

    let mut email_addrs = HashSet::new();

    if group_pending.is_some() {
        for email in ganbare::email::get_all_pending_email_confirms(&conn).err_500()? {
            email_addrs.insert(email);
        }
    }

    for g in group {
        for (u, _) in user::get_users_by_group(&conn, g).err_500()? {
            if let Some(email) = u.email {
                email_addrs.insert(email);
            }
        }
    }

    ganbare::email::send_freeform_email(&*EMAIL_SERVER,
                                        &*EMAIL_SMTP_USERNAME,
                                        &*EMAIL_SMTP_PASSWORD,
                                        (&*EMAIL_ADDRESS, &*EMAIL_NAME),
                                        email_addrs.iter().map(|s| &**s),
                                        &subject,
                                        &body).err_500()?;

    redirect("/send_mail?sent=true", 303).refresh_cookie(&sess)
}

pub fn events(req: &mut Request) -> PencilResult {
    let (_, _, sess) = auth_user(req, "editors")?;

    let context = new_template_context();

    req.app
        .render_template("events.html", &context)
        .refresh_cookie(&sess)
}

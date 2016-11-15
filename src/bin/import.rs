#![feature(field_init_shorthand)]

extern crate ganbare;

#[macro_use]
extern crate clap;
extern crate dotenv;
extern crate mime;
extern crate unicode_normalization;

use unicode_normalization::UnicodeNormalization;
use ganbare::*;

fn import_batch(path: &str) {
    let conn = db_connect().unwrap();

    let files = std::fs::read_dir(path).unwrap();

    for f in files {
        let f = f.unwrap();
        let path = f.path();
        let extension = if let Some(Some(e)) = path.extension().map(|s|s.to_str()) { e } else { continue }.to_string();
        let file_name = path.file_name().unwrap().to_str().unwrap().to_string();
        let word = path.file_stem().unwrap().to_str().unwrap().nfc().collect::<String>();
        let containing_dir = path.parent().unwrap().strip_prefix(
                                                                    path.parent().unwrap().parent().unwrap()
                                                                ).unwrap().to_str().unwrap().to_string();

        if extension != "mp3" { continue };

        use std::str::FromStr;
        let mime = mime::Mime::from_str("audio/mpeg").unwrap();

        let nugget = word.replace("*", "").replace("・", "");

        let w = NewWordFromStrings {
            word,
            explanation: "".into(),
            nugget,
            narrator: containing_dir,
            files: vec![(path, Some(file_name), mime)],
        };
        
        println!("{:?}", w);

        create_word(&conn, w).unwrap();
    }

}


fn main() {
    use clap::*;

    let matches = App::new("ganba.re word import tool")
        .arg(Arg::with_name("PATH")
                                .index(1)
                                .required(true)
                                .value_name("PATH")
                                .help("The path to the input files")
                                .takes_value(true))
        .version(crate_version!())
        .get_matches();
    
    let path = matches.value_of("PATH").unwrap();

    import_batch(&path);
}

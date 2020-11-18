//
// This is a simple command line tool that allows one to make symlinks for
// jpeg files based on the date information in those files, turning a
// shamble of random files into a single, date-based hierarchy of files,
// suitable for simple browsing, searching, etc.
//
// (FYI: This is my first Rust program)
//
// Author: Phil Shafer <phil@>
// This code is hereby in the public domain.  Caveat you!
//

#[macro_use]
extern crate lazy_static;

use std::path::{Path, PathBuf};
use std::io::{BufRead, BufReader};
use std::fs::File;
use std::fs;
use std::ffi::OsString;

use clap::{App, Arg};
use snafu::{ResultExt, Snafu};
use exif::Exif;
use regex::Regex;

#[derive(Debug, Snafu)]
enum Error {
    #[snafu(display("Could not open input file '{}': {}",
                    path.display(), source))]
    FileError {
        path: PathBuf,
        source: std::io::Error,
    },
    #[snafu(display("Could not parse input file '{}': {}",
                    path.display(), source))]
    ParseError {
        path: PathBuf,
        source: exif::Error,
    },
    #[snafu(display("missing field in input file '{}': {}",
                    path.display(), field))]
    MissingField {
        path: PathBuf,
        field: String,
    },
    #[snafu(display("file is too small (thumbnail?) '{}': {}",
                    path.display(), len))]
    TooSmall {
        path: PathBuf,
        len: u64,
    },
}

//
// This is a set of JPEG attributes we use to find the date of the photo.
//
lazy_static! {
    static ref DATETIME: Vec<exif::Tag>
        = vec![exif::Tag::DateTimeDigitized,
               exif::Tag::DateTimeOriginal,
               exif::Tag::DateTime,
        ];
}

lazy_static! {
    static ref DATE_REGEX : Regex = Regex::new(
        r"(?P<y>\d{4})-(?P<m>\d{2})-(?P<d>\d{2}) (?P<H>\d{2}):(?P<M>\d{2}):(?P<S>\d{2})"
    ).unwrap();
}

fn main() {
    let args = App::new("imagelink")
        .version("0.1.0")
        .author("phil@")
        .about("Links images based on date")
        .arg(
            Arg::with_name("file")
                .multiple(true)
                .help("name of file to link"),
        )
        .arg(
            Arg::with_name("list")
                .short("l")
                .long("list")
                .multiple(true)
                .takes_value(true)
                .help("name of file contains files to link"),
        )
        .arg(
            Arg::with_name("base")
                .short("b")
                .long("base")
                .takes_value(true)
                .help("Base directory for output links"),
        )
        .arg(
            Arg::with_name("no-execute")
                .short("n")
                .long("no-execute")
                .help("Don't really make links"),
        )
        .get_matches();

    let base = args.value_of("base");
    println!("base is {}", base.unwrap_or("."));

    let mut files: Vec<PathBuf> = match args.values_of_os("file") {
        None => vec![],
        Some(f) => f.map(|p| PathBuf::from(p)).collect(),
    };
    let lists: Vec<PathBuf> = match args.values_of_os("list") {
        None => vec![],
        Some(f) => f.map(|p| PathBuf::from(p)).collect(),
    };

    for list in lists {
        let file = File::open(&list).unwrap();
        println!("list: {:?}", list);
        for line in BufReader::new(file).lines() {
            let line = line.unwrap();
            println!("{}", line);
            files.push(PathBuf::from(&line.trim()));
        }
    }

    println!("The file passed is: {:?}", files);

    let no_execute = args.is_present("no-execute");
    if no_execute {
        println!("not executing...");
    } else {
        println!("executing...");
    }

    'file: for file in files {
        println!("working: {:?}", file);

        let mut source = PathBuf::new();
        source.push("../../..");
        if let Some(b) = base {
            source.push(b);
        }
        source.push(&file);
        let src = source.as_path();

        let targ = {
            match link_name(&file) {
                Ok(targ) => targ,
                Err(e) => { println!("error: {:?}", e); continue 'file; }
            }
        };

        println!("linking {:?} to {:?} ... ", src, targ);

        if let Some(parent) = targ.parent() {
            if !parent.exists() {
                println!("mkdir -p {:?}", parent);
                if !no_execute {
                    if let Err(e) = fs::create_dir_all(parent) {
                        println!("error: {:?}", e);
                    }
                }
            }
        }

        println!("ln -s {:?} {:?}", src, targ);
        if !no_execute {
            use std::os::unix::fs;

            if let Err(e) = fs::symlink(src, targ) {
                println!("error: {:?}", e);
            }                
        }
    }
}

fn link_name (path: &Path) -> Result<PathBuf, Error> {
    let file = File::open(path).context(FileError { path })?;

    if let Ok(i) = file.metadata() {
        if i.len() < 100 * 1024 {
            return Err(Error::TooSmall { path: path.to_path_buf(),
                                         len: i.len() });
        }
    }

    let mut bufreader = std::io::BufReader::new(&file);
    let exifreader = exif::Reader::new();
    let exif = exifreader.read_from_container(&mut bufreader)
        .context(ParseError{ path })?;
    
    for f in exif.fields() {
        println!("'{}' [{}] :: '{}'",
                 f.tag, f.ifd_num, f.display_value().with_unit(&exif));
    }

    let datetime = first_of(path, &exif, &DATETIME)?;
    println!("datetime '{}'", datetime);

    let res = DATE_REGEX.replace_all(&datetime, "$y/$m/$d/$H-$M-$S-");
    println!("datetime '{}'", res);
    
    let mut target = OsString::new();
    target.push(res.to_string());

    let s = {
        match path.file_name() {
            Some(f) => f.to_string_lossy(),
            None => path.to_string_lossy(),
        }
    };
    let s2 = s.replace(" ", "-");
    target.push(PathBuf::from(s2));

    println!("target '{:?}", target);

    Ok(PathBuf::from(target))
}

//
/// Look through the EXIF data for a set of fields, returning the first one.
/// Tags are exif::Tag::* values (e.g. exif::Tag::DateTimeDigitized).
//
fn first_of (path: &Path, exif: &Exif, tags: &Vec<exif::Tag>)
             -> Result<String, Error> {
    for tag in tags {
        if let Some(value) = exif.get_field(*tag, exif::In::PRIMARY) {
            return Ok(value.display_value().to_string())
        }
    }

    // None of the fields were found, so we pick the first field
    // as the one to complain about
    Err(Error::MissingField { path: path.to_path_buf(),
                              field: tags[0].description()
                              .unwrap_or("[unknown]").to_string() })
}

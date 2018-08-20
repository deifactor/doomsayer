#[macro_use]
extern crate failure;
extern crate mammut;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate slog;
extern crate slog_async;
extern crate slog_term;
#[macro_use]
extern crate structopt;

use slog::Drain;
use std::fmt;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::path::PathBuf;
use structopt::StructOpt;

/// Application state that needs to persist between runs.
#[derive(Serialize, Deserialize)]
struct State {
    /// Mastodon access token.
    access_token: mammut::Data,
    /// Index of the last toot that we successfully posted. If this is not set,
    /// we haven't made any toots. If this is equal to the number of toots,
    /// there's nothing for us to do.
    last_successful_toot: Option<usize>,
}

// Custom debug implementation so we don't leak toots in logs. Because that
// would be bad.
impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "State {{ access_token: [REDACTED], last_successful_toot: {:?}}}",
            self.last_successful_toot
        )
    }
}

impl State {
    fn register() -> Result<(), failure::Error> {
        let app = mammut::apps::AppBuilder {
            client_name: "doomsayer",
            redirect_uris: "urn:ietf:wg:oauth:2.0:oob",
            scopes: mammut::apps::Scopes::Write,
            website: Some("https://github.com/deifactor/doomsayer"),
        };
        // XXX: don't hard-code this
        let mut registration = mammut::Registration::new("https://botsin.space");
        registration.register(app)?;
        let auth_url = registration.authorise()?;
        println!("Visit this link while logged in as the bot: {}", auth_url);
        print!("Paste the code you got from your instance: ");
        io::stdout().flush()?;
        let mut code = String::new();
        io::stdin().read_line(&mut code)?;
        registration.create_access_token(code.to_string())?;
        Ok(())
    }
}

#[derive(StructOpt, Debug)]
#[structopt(name = "doomsayer")]
struct Opt {
    /// The file to store doomsayer's state in. Does not need to exist.
    #[structopt(short = "s", long = "state", parse(from_os_str))]
    state: PathBuf,

    /// The text file containing all of the strings to post.
    #[structopt(short = "t", long = "toots", parse(from_os_str))]
    toots: PathBuf,
}

fn main() -> Result<(), failure::Error> {
    let opt = Opt::from_args();
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let log = slog::Logger::root(drain, o!());

    info!(log, "Loading state from {:?}", &opt.state);
    let state: State = match File::open(&opt.state) {
        Ok(f) => serde_json::from_reader(f)?,
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => {
            info!(log, "State file not found at {:?}", &opt.state);
            State::register()?;
            info!(
                log,
                "Registration successful. doomsayer will post on the next run."
            );
            return Ok(());
        }
        Err(e) => bail!(e),
    };

    info!(log, "Reading the next toot from {:?}", &opt.toots);
    let toots = io::BufReader::new(File::open(&opt.toots)?);
    let toot_index = state.last_successful_toot.map(|n| n + 1).unwrap_or(0);
    if let Some(maybe_line) = toots.lines().nth(toot_index) {
        match maybe_line {
            Ok(line) => {
                info!(log, "Tooting {:?}", line);
                let builder = mammut::status_builder::StatusBuilder::new(line);
                let toot =
                    mammut::Mastodon::from_data(state.access_token.clone()).new_status(builder)?;
                info!(log, "Toot successful: {}", toot.uri);
            }
            Err(e) => {
                error!(log, "Could not read toot: {:?}", e);
                bail!(e)
            }
        }
    } else {
        info!(log, "All out of toots");
    }

    let file = File::create(&opt.state)?;
    let state = State {
        last_successful_toot: Some(toot_index),
        ..state
    };
    serde_json::to_writer_pretty(file, &state)?;
    Ok(())
}

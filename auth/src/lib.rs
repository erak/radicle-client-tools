#![allow(clippy::or_fun_call)]
use std::ffi::OsString;
use std::str::FromStr;

use anyhow::Context as _;
use radicle_common::signer::ToSigner;
use zeroize::Zeroizing;

use librad::crypto::keystore::pinentry::SecUtf8;
use librad::profile::ProfileId;

use radicle_common::args::{Args, Error, Help};
use radicle_common::{config, git, keys, person, profile};
use radicle_terminal as term;

pub const HELP: Help = Help {
    name: "auth",
    description: env!("CARGO_PKG_DESCRIPTION"),
    version: env!("CARGO_PKG_VERSION"),
    usage: r#"
Usage

    rad auth [--init | --active] [<options>...] [<profile>]

    If `--init` is used, name and passphrase may be given via the `--name`
    and `--passphrase` option. Using these disables the respective input prompt.

Options

    --init                  Initialize a new identity
    --active                Authenticate with the currently active profile
    --name <name>           Use given name (default: none)
    --passphrase <phrase>   Use given passphrase (default: none)
    --help                  Print help
"#,
};

#[derive(Debug)]
pub struct Options {
    pub init: bool,
    pub active: bool,
    pub name: Option<String>,
    pub passphrase: Option<String>,
    pub profile: Option<ProfileId>,
}

impl Args for Options {
    fn from_args(args: Vec<OsString>) -> anyhow::Result<(Self, Vec<OsString>)> {
        use lexopt::prelude::*;

        let mut init = false;
        let mut active = false;
        let mut name = None;
        let mut passphrase = None;
        let mut profile = None;
        let mut parser = lexopt::Parser::from_args(args);

        while let Some(arg) = parser.next()? {
            match arg {
                Long("init") => {
                    init = true;
                }
                Long("active") => {
                    active = true;
                }
                Long("name") if init && name.is_none() => {
                    let val = parser
                        .value()?
                        .to_str()
                        .ok_or(anyhow::anyhow!("invalid name specified with `--name`"))?
                        .to_owned();

                    name = Some(val);
                }
                Long("passphrase") if init && passphrase.is_none() => {
                    let val = parser
                        .value()?
                        .to_str()
                        .ok_or(anyhow::anyhow!(
                            "invalid passphrase specified with `--passphrase`"
                        ))?
                        .to_owned();

                    term::warning(
                        "Passing a plain-text passphrase is considered insecure. \
                        Please only use for testing purposes.",
                    );

                    passphrase = Some(val);
                }
                Long("help") => {
                    return Err(Error::Help.into());
                }
                Value(val) => {
                    let string = val.to_str().ok_or_else(|| {
                        anyhow::anyhow!("invalid UTF-8 string specified for profile")
                    })?;
                    let id = ProfileId::from_str(string).context("invalid profile id specified")?;

                    profile = Some(id);
                }
                _ => return Err(anyhow::anyhow!(arg.unexpected())),
            }
        }

        Ok((
            Options {
                init,
                active,
                name,
                passphrase,
                profile,
            },
            vec![],
        ))
    }
}

pub fn run(options: Options, ctx: impl term::Context) -> anyhow::Result<()> {
    let profiles = match profile::list() {
        Ok(profiles) => profiles,
        _ => vec![],
    };

    if options.init || profiles.is_empty() {
        if options.profile.is_some() {
            anyhow::bail!("you may not specify a profile id when initializing a new identity");
        }
        init(options)
    } else {
        authenticate(&profiles, options, ctx)
    }
}

pub fn init(options: Options) -> anyhow::Result<()> {
    term::headline("Initializing your 🌱 profile and identity");

    let sock = keys::ssh_auth_sock();

    if git::check_version().is_err() {
        term::warning(&format!(
            "Your git version is unsupported, please upgrade to {} or later",
            git::VERSION_REQUIRED,
        ));
        term::blank();
    }

    let name = sanitize_name(
        options
            .name
            .unwrap_or_else(|| term::text_input("Name", None).unwrap()),
    )?;
    let passphrase = options
        .passphrase
        .map_or_else(term::secret_input_with_confirmation, |passphrase| {
            SecUtf8::from(passphrase)
        });
    let pwhash = keys::pwhash(passphrase.clone());
    let home = profile::home();

    let mut spinner = term::spinner("Creating your 🌱 Ed25519 keypair...");
    let (profile, peer_id) = profile::create(home, pwhash.clone())?;

    let signer = if let Ok(sock) = sock {
        spinner.finish();
        spinner = term::spinner("Adding to ssh-agent...");

        keys::add(&profile, pwhash, sock.clone())?;
        let signer = sock.to_signer(&profile)?;

        spinner.finish();
        signer
    } else {
        let signer = keys::load_secret_key(&profile, passphrase)?.to_signer(&profile)?;

        spinner.finish();
        signer
    };

    spinner = term::spinner("Setting up config...");
    config::Config::init(&profile)?;
    spinner.finish();

    let storage = keys::storage(&profile, signer.clone())?;
    let person = person::create(&profile, &name, signer, &storage)
        .context("could not create identity document")?;
    person::set_local(&storage, &person)?;

    term::success!(
        "Profile {} created.",
        term::format::highlight(&profile.id().to_string())
    );

    term::blank();
    term::info!(
        "Your radicle Peer ID is {}. This identifies your device.",
        term::format::highlight(&peer_id.to_string())
    );
    term::info!(
        "Your personal 🌱 URN is {}. This identifies you across devices.",
        term::format::highlight(&person.urn().to_string())
    );

    term::blank();
    term::tip!(
        "To create a radicle project, run {} from a git repository.",
        term::format::secondary("`rad init`")
    );

    Ok(())
}

pub fn authenticate(
    profiles: &[profile::Profile],
    options: Options,
    ctx: impl term::Context,
) -> anyhow::Result<()> {
    let profile = match ctx.profile() {
        Ok(profile) => profile,
        Err(_) => {
            anyhow::bail!(
                "Active profile could not be loaded.\n\
                To create a new profile, run `rad auth --init`."
            )
        }
    };

    if !options.active && options.profile.is_none() {
        term::info!(
            "Your active profile is {}",
            term::format::highlight(&profile.id().to_string()),
        );
    }

    let selection = if let Some(id) = options.profile {
        profiles
            .iter()
            .find(|p| p.id() == &id)
            .ok_or_else(|| anyhow::anyhow!("profile '{}' not found", id))?
    } else if profiles.len() > 1 && !options.active {
        if let Some(p) = term::profile_select(profiles, &profile) {
            p
        } else {
            return Ok(());
        }
    } else {
        &profile
    };

    let read_only = profile::read_only(selection)?;
    let config = read_only.config()?;

    if let Some(user) = config.user()? {
        let username = config.user_name()?;

        term::headline(&format!(
            "🌱 Authenticating as {} {}",
            term::format::highlight(user),
            term::format::dim(format!("({})", username))
        ));
    }

    if selection.id() != profile.id() {
        let id = selection.id();
        profile::set(id)?;

        term::success!("Profile {} activated", id);
    }

    let profile = selection;
    if let Ok(sock) = keys::ssh_auth_sock() {
        if !keys::is_ready(profile, sock.clone())? {
            term::warning("Adding your radicle key to ssh-agent...");

            // TODO: We should show the spinner on the passphrase prompt,
            // otherwise it seems like the passphrase is valid even if it isn't.
            let secret_input: SecUtf8 = if atty::is(atty::Stream::Stdin) {
                term::secret_input()
            } else {
                let mut input: Zeroizing<String> = Zeroizing::new(Default::default());
                std::io::stdin().read_line(&mut input)?;
                SecUtf8::from(input.trim_end())
            };
            let pass = keys::pwhash(secret_input);
            let spinner = term::spinner("Unlocking...");

            keys::add(profile, pass, sock).context("invalid passphrase supplied")?;
            spinner.finish();

            term::success!("Radicle key added to ssh-agent");
        } else {
            term::success!("Signing key already in ssh-agent");
        }
    }

    Ok(())
}

fn sanitize_name(name: String) -> anyhow::Result<String> {
    if name.contains(char::is_whitespace) {
        anyhow::bail!("Name cannot contain whitespaces");
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use assay::assay;

    use super::*;

    use radicle_common::test;

    fn create_auth_options(name: &str) -> Options {
        Options {
            active: false,
            init: true,
            name: Some(name.to_owned()),
            passphrase: Some(test::USER_PASS.to_owned()),
            profile: None,
        }
    }

    #[assay(
        setup = test::setup::lnk_home()?,
        teardown = test::teardown::profiles()?,
    )]
    fn can_be_initialized() {
        let options = create_auth_options("user");

        init(options).unwrap();

        assert_eq!(profile::count().unwrap(), 1);
        assert_eq!(profile::name(None).unwrap(), "user");
    }

    #[assay(
        setup = test::setup::lnk_home()?,
    )]
    fn name_cannot_contain_whitespace() {
        let options = create_auth_options("user A");

        assert!(init(options).is_err());
    }
}

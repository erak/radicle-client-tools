use anyhow::{anyhow, Context as _, Error, Result};

use librad::crypto::keystore::crypto::Pwhash;
use librad::crypto::BoxedSigner;
use librad::git::storage::Storage;

use librad::profile::{Profile, ProfileId};
use librad::PeerId;

use rad_clib::keys;
use rad_clib::keys::ssh::SshAuthSock;
use rad_clib::storage;
use rad_clib::storage::ssh;

use rad_terminal::keys::CachedPrompt;

pub fn storage(profile: &Profile, sock: SshAuthSock) -> Result<(BoxedSigner, Storage), Error> {
    match ssh::storage(profile, sock) {
        Ok(result) => Ok(result),
        Err(storage::Error::SshKeys(keys::ssh::Error::NoSuchKey(_))) => Err(anyhow!(
            "the radicle ssh key for this profile is not in ssh-agent"
        )),
        Err(err) => Err(anyhow!(err)),
    }
}

pub fn add(
    profile: &Profile,
    pass: Pwhash<CachedPrompt>,
    sock: SshAuthSock,
) -> Result<ProfileId, Error> {
    rad_profile::ssh_add(None, profile.id().clone(), sock, pass, &Vec::new())
        .context("could not add ssh key")
}

pub fn is_ready(profile: &Profile, sock: SshAuthSock) -> Result<bool, Error> {
    rad_profile::ssh_ready(None, profile.id().clone(), sock)
        .context("could not lookup ssh key")
        .map(|(_, is_ready)| is_ready)
}

/// Get the SSH long key from a peer id.
/// This is the output of `ssh-add -L`.
pub fn to_ssh_key(peer_id: &PeerId) -> Result<String, std::io::Error> {
    use byteorder::{BigEndian, WriteBytesExt};

    let mut buf = Vec::new();
    let key = peer_id.as_public_key().as_ref();
    let len = key.len();

    buf.write_u32::<BigEndian>(len as u32)?;
    buf.extend_from_slice(key);

    // Despite research, I have no idea what this string is, but it seems
    // to be the same for all Ed25519 keys.
    let mut encoded = String::from("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5");
    encoded.push_str(&base64::encode(buf));

    Ok(encoded)
}
//! Pinned launcher protocol: runtime directory, writable directory, executable,
//! then unchanged command arguments. The caller sets the working directory and
//! supplies the output descriptors before this program starts.
use crate::supervision::{Error, Profile};
use std::{ffi::OsString, os::unix::process::CommandExt, path::Path, process::Command};

fn command(mut arguments: impl Iterator<Item = OsString>) -> Result<Command, Error> {
    let runtime = arguments.next().ok_or(Error::InvalidRequest)?;
    let work = arguments.next().ok_or(Error::InvalidRequest)?;
    let executable = arguments.next().ok_or(Error::InvalidRequest)?;
    if !crate::runtime::normal_absolute(Path::new(&executable)) {
        return Err(Error::InvalidRequest);
    }
    let profile = Profile::for_run(
        rustix::process::geteuid().as_raw(),
        Path::new(&runtime),
        Path::new(&work),
    )?;
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .env_clear()
        .envs(profile.environment());
    Ok(command)
}
pub fn execute(
    arguments: impl Iterator<Item = OsString>,
) -> Result<std::convert::Infallible, Error> {
    let _ = command(arguments)?.exec();
    Err(Error::Unavailable)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clears_environment_and_preserves_paths_and_every_argument() {
        let args = [
            "/runtime=a",
            "/work dir",
            "/runtime=a/bin/task=b",
            "--name",
            "two words",
            "$HOME %i; *",
            "",
            "a=b",
        ];
        let cmd = command(args.map(OsString::from).into_iter()).unwrap();
        assert_eq!(cmd.get_program(), args[2]);
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), &args[3..]);
        let env = cmd
            .get_envs()
            .map(|(key, value)| (key.to_str().unwrap(), value.unwrap().to_str().unwrap()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(env.len(), 7);
        assert_eq!(env["PATH"], "/runtime=a/bin");
        assert_eq!(env["HOME"], "/work dir");
        assert_eq!(env["TMPDIR"], "/work dir");
        assert_eq!(env["TZ"], "UTC");
        assert_eq!(env["LANG"], "C.UTF-8");
    }
    #[test]
    fn rejects_incomplete_or_unsafe_launcher_profiles() {
        for args in [
            vec![],
            vec!["/runtime"],
            vec!["/runtime", "/work"],
            vec!["relative", "/work", "/task"],
            vec!["/runtime", "/work", "relative"],
            vec!["/runtime:other", "/work", "/task"],
        ] {
            assert!(command(args.into_iter().map(OsString::from)).is_err());
        }
        for (uid, runtime, work, launcher) in [
            (0, "/runtime", "/work", "/launcher"),
            (1000, "/runtime", "relative", "/launcher"),
            (1000, "/runtime", "/work", "relative"),
            (1000, "/runtime", "/work", "/bad\0launcher"),
            (1000, "/bad\0runtime", "/work", "/launcher"),
        ] {
            assert!(
                Profile::with_launcher(
                    uid,
                    Path::new(runtime),
                    Path::new(work),
                    Path::new(launcher)
                )
                .is_err()
            );
        }
    }
}

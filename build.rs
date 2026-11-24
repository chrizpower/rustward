use std::{env, fs, io, path::Path, process::Command};

fn npm(project: &Path, args: &[&str]) {
    let status = Command::new("npm")
        .args(args)
        .current_dir(project)
        .status()
        .expect("Install Node.js and npm to build the Linux web console");
    assert!(status.success(), "npm {} failed", args.join(" "));
}

fn copy_source(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let destination = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_source(&entry.path(), &destination)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

fn build_console(project: &Path) -> io::Result<()> {
    fs::create_dir_all(project)?;
    let mut install = !project.join("node_modules/.bin/vite").is_file();
    for name in ["package.json", "package-lock.json"] {
        let source = Path::new("web").join(name);
        let target = project.join(name);
        let data = fs::read(source)?;
        install |= fs::read(&target).ok().as_ref() != Some(&data);
        fs::write(target, data)?;
    }
    let ready = project.join("dependencies-ready");
    if install || !ready.is_file() {
        if ready.exists() {
            fs::remove_file(&ready)?;
        }
        npm(project, &["ci", "--include=dev", "--no-audit", "--no-fund"]);
        fs::write(ready, "")?;
    }

    for name in ["index.html", "tsconfig.json", "vite.config.ts"] {
        fs::copy(Path::new("web").join(name), project.join(name))?;
    }
    let source = project.join("src");
    if source.exists() {
        fs::remove_dir_all(&source)?;
    }
    copy_source(Path::new("web/src"), &source)?;
    npm(project, &["run", "build"]);
    Ok(())
}

fn set_version() {
    println!("cargo:rerun-if-env-changed=RUSTWARD_SOURCE_REVISION");
    let revision = env::var("RUSTWARD_SOURCE_REVISION").unwrap_or_default();
    let build = if revision.is_empty() {
        "local".to_owned()
    } else {
        assert!(
            revision.len() == 40
                && revision
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "RUSTWARD_SOURCE_REVISION must be a full lowercase Git commit SHA"
        );
        format!("g{revision}")
    };
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let (base, metadata) = version.split_once('+').unwrap_or((&version, ""));
    let separator = if base.contains('-') { "." } else { "-dev." };
    let metadata = if metadata.is_empty() {
        String::new()
    } else {
        format!("+{metadata}")
    };
    println!("cargo:rustc-env=RUSTWARD_VERSION={base}{separator}{build}{metadata}");
}

fn main() {
    set_version();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        println!("cargo:rerun-if-changed=build.rs");
        return;
    }
    for path in [
        "web/package.json",
        "web/package-lock.json",
        "web/index.html",
        "web/tsconfig.json",
        "web/vite.config.ts",
        "web/src",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let out = env::var_os("OUT_DIR").expect("Cargo build output directory");
    build_console(&Path::new(&out).join("console")).expect("Build the web console");
}

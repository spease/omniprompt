use anyhow::{Context, Result, anyhow};
use chrono::Local;
use core::str::FromStr;
#[cfg(feature="git")]
use git2::Repository;
use std::io::Write;
use std::net::IpAddr;
use std::path::Path;

mod colors {
    use std::fmt::Display;
    use std::path::Path;
    use std::ffi::OsStr;

    thread_local! {
        static ESCAPES: (&'static str, &'static str) = {
            let ppid = std::os::unix::process::parent_id();
            Path::new(&format!("/proc/{ppid}/exe"))
                .read_link()
                .ok()
                .and_then(|p| {
                    if p.file_name() == Some(OsStr::new("zsh")) {
                        Some(("\x25\x7b", "\x25\x7d"))
                    } else if p.file_name() == Some(OsStr::new("bash")) {
                        Some((r#"\["#, r#"\]"#))
                    } else {
                        None
                    }
                })
                .unwrap_or(("",""))
        }
    }

    macro_rules! def_colors {
        ($($color_name:ident | $color_name_lower: ident => ($color:literal, $reset:literal)),+) => {
            $(
                pub struct $color_name<T: Display>(T);

                impl<T: Display> Display for $color_name<T> {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        ESCAPES.with(|(escape_begin, escape_end)| {
                            write!(f, concat!("{}", "\x1b[", $color, "m", "{}{}{}", "\x1b[", $reset, "m", "{}"), escape_begin, escape_end, self.0, escape_begin, escape_end)
                            /*
                            if supports_color::on_cached(supports_color::Stream::Stdout).is_some() {
                                write!(f, concat!("{}", "\x1b[", $color, "m", "{}{}{}", "\x1b[", $reset, "m", "{}"), escape_begin, escape_end, self.0, escape_begin, escape_end)
                            } else {
                                self.0.fmt(f)
                            }
                            */
                        })
                    }
                }
            )+

            pub trait Colorizer {
                type Target: Display;
                $(
                    fn $color_name_lower(self) -> $color_name<Self::Target>;
                )+
            }

            impl<T> Colorizer for T where T: Display {
                type Target = T;

                $(
                    fn $color_name_lower(self) -> $color_name<Self::Target> {
                        $color_name(self)
                    }
                )+
            }
        }
    }

    def_colors! {
        Bold | bold => (1, 22),
        Red | red => (31, 39),
        Green | green => (32, 39),
        Yellow | yellow => (33, 39),
        Blue | blue => (34, 39),
        Magenta | magenta => (35, 39),
        Cyan | cyan => (36, 39)
    }

}

use colors::Colorizer;

macro_rules! let_workaround {
    (let $name:ident = $val:expr; $($rest:tt)+) => {
        match $val {
            $name => {
                let_workaround! { $($rest)+ }
            }
        }
    };
    ($($rest:tt)+) => { $($rest)+ }
}

struct FieldWriter<T: Write> {
    column_count: usize,
    errors: String,
    exit_code: Option<i32>,
    row_count: usize,
    stream: T,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum Field {
    ExitCode,
    #[cfg(feature="git")]
    Git,
    #[cfg(feature="network")]
    Network,
    #[cfg(feature="platform")]
    Platform,
    Ppid,
    Prompt,
    Pwd,
    Time,
    #[cfg(feature="tty")]
    Tty,
    Whoami,
}

impl<T: Write> FieldWriter<T> {
    fn new(stream: T, exit_code: Option<i32>) -> Self {
        Self {
            column_count: 0,
            errors: String::new(),
            exit_code,
            row_count: 0,
            stream,
        }
    }

    fn print_line(&mut self) -> Result<()> {
        writeln!(self.stream)?;
        self.column_count = 0;
        self.row_count += 1;
        Ok(())
    }

    fn print_field(function: Field, exit_code: Option<i32>, stream: &mut T) -> Result<()> {
        #[cfg(any(not(unix)))]
        let si = {
            use sysinfo::{RefreshKind};
            let mut rk = RefreshKind::new();
            {
                use sysinfo::ProcessRefreshKind;
                rk = rk.with_processes(ProcessRefreshKind::new());
            }
            sysinfo::System::new_with_specifics(rk)
        };
        match function {
            Field::ExitCode => {
                match exit_code {
                    Some(0) => write!(stream, "{}", 0.green().bold())?,
                    Some(v) => write!(stream, "{}", v.red().bold())?,
                    None => {},
                }
            }
            #[cfg(feature="git")]
            Field::Git => {
                if let Ok(repo) = Repository::discover(".") {
                    write!(stream, "{}", repo.head().context("trying to get HEAD")?.shorthand().unwrap_or("<UNKNOWN>").yellow())?;
                }
            },
            #[cfg(feature="network")]
            Field::Network => {
                use bytesize::ByteSize;
                let (upload, download) = sysinfo::Networks::new_with_refreshed_list().into_iter().map(|(_, nw)| (ByteSize(nw.received()), ByteSize(nw.transmitted()))).fold((ByteSize(0),ByteSize(0)), |sum,current|(sum.0+current.0, sum.1+current.1));
                write!(stream, "↑{}↓{}", upload, download)?;
            },
            #[cfg(feature="platform")]
            Field::Platform => {
                #[cfg(unix)]
                if let Some(os_version) = sysinfo::System::os_version() {
                    write!(
                        stream,
                        "{}",
                        format_args!(
                            "{} ({})/{}/{}",
                            sysinfo::System::distribution_id(),
                            os_version,
                            nix::sys::utsname::uname()?.release().to_string_lossy(),
                            std::env::consts::ARCH
                        ).red()
                    )?;
                } else {
                    write!(
                        stream,
                        "{}",
                        format_args!(
                            "{}/{}",
                            nix::sys::utsname::uname()?.release().to_string_lossy(),
                            std::env::consts::ARCH
                        ).red()
                    )?;
                }
                #[cfg(not(unix))]
                if let Some(os_version) = sysinfo::System::os_version() {
                    write!(
                        stream,
                        "{}",
                        format_args!(
                            "{} ({})/{}",
                            sysinfo::System::distribution_id(),
                            os_version
                            std::env::consts::ARCH
                        ).red()
                    )?;
                } else {
                    write!(
                        stream,
                        "{}",
                        format_args!(
                            "{}",
                            std::env::consts::ARCH
                        ).red()
                    )?;
                }
            },
            Field::Ppid => {
                #[cfg(unix)]
                write!(stream, "{}", std::os::unix::process::parent_id().yellow())?;
                #[cfg(not(unix))]
                {
                    use sysinfo::{ProcessExt, SystemExt};
                    let pid = sysinfo::get_current_pid().map_err(|e|anyhow!("{}",e))?;
                    let parent_pid = si.process(pid).ok_or_else(||anyhow!("Couldn't find current PID"))?.parent().ok_or_else(||anyhow!("No parent for current process"))?;
                    write!(stream, "{}", parent_pid.yellow())?;
                }
            }
            Field::Prompt => {
                write!(stream, "{}", "$".magenta().bold())?;
            }
            Field::Pwd => {
                let cwd = std::env::current_dir()?;
                let final_path = match dirs::home_dir() {
                    Some(home_dir) => match cwd.strip_prefix(home_dir) {
                        Ok(relpath) if !relpath.as_os_str().is_empty() => Path::new("~").join(relpath),
                        Ok(_) => "~".into(),
                        Err(_) => cwd,
                    },
                    None => cwd,
                };
                write!(stream, "{}", final_path.display().yellow().bold())?;
            }
            Field::Time => {
                // stream.write_all(Local::now().to_rfc3339().as_bytes())?;
                write!(stream, "{}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f %Z").magenta())?;
            }
            #[cfg(feature="tty")]
            Field::Tty => {
                use std::os::unix::io::AsRawFd;
                let stdin_fd = std::io::stdin().as_raw_fd();
                write!(stream, "{}", nix::unistd::ttyname(stdin_fd)?.to_string_lossy().yellow())?;
            }
            Field::Whoami => {
                let_workaround! {
                    let first = format_args!(
                        "{}@{}",
                        whoami::username().bold(),
                        whoami::fallible::hostname().unwrap_or_else(|_|String::from("???")).bold()
                    );
                    if let Some(ssh_connection) = std::env::var_os("SSH_CONNECTION") {
                        let mut pieces = ssh_connection.to_str().ok_or_else(||anyhow!("Invalid UTF-8 for SSH_CONNECTION"))?.split(' ').skip(2);
                        let ssh_server_ip = IpAddr::from_str(pieces.next().ok_or_else(||anyhow!("Missing server IP"))?)?;
                        let ssh_server_port = u16::from_str(pieces.next().ok_or_else(||anyhow!("Missing server port"))?)?;

                        write!(stream, "{}", format_args!("{} ({}:{})", first, ssh_server_ip, ssh_server_port).cyan())?;
                    } else {
                        write!(stream, "{}", first.cyan())?;
                    }
                }
            }
        }

        Ok(())
    }

    fn print_section(&mut self, function: Field) -> Result<()> {
        if self.column_count != 0 {
            self.stream.write_all(if self.row_count == 0 { b" - " } else { b"-" })?;
        }
        write!(self.stream, "{}", (if self.column_count != 0 { "[" } else if self.row_count == 0 { "┌─[" } else { "└─[" }).blue().bold())?;

        if let Err(e) = Self::print_field(function, self.exit_code, &mut self.stream) {
            use std::fmt::Write;
            if self.errors.is_empty() {
                write!(self.errors, "{:?}", e)?;
            } else {
                write!(self.errors, "\n{:?}", e)?;
            }
        }
        self.column_count += 1;

        write!(self.stream, "{}", (if function != Field::Prompt { "]" } else { "]> " }).blue().bold())?;

        Ok(())
    }

    fn print_errors(&mut self) -> Result<()> {
        write!(self.stream, "{}", (&self.errors).red().bold())?;
        Ok(())
    }

    fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

fn print_default(exit_code: Option<i32>) -> Result<()> {
    let mut out = [0u8; 1024];
    let out_len = out.len() - {
        let mut out_written = &mut out[..];
        // let stdout = std::io::stdout();
        // let mut fw = FieldWriter::new(stdout.lock(), exit_code);
        let mut fw = FieldWriter::new(&mut out_written, exit_code);

        fw.print_section(Field::Whoami)?;
        fw.print_section(Field::Pwd)?;
        fw.print_section(Field::Ppid)?;
        fw.print_section(Field::Time)?;
        #[cfg(feature="platform")]
        fw.print_section(Field::Platform)?;
        #[cfg(feature="network")]
        fw.print_section(Field::Network)?;
        fw.print_line()?;
        fw.print_section(Field::ExitCode)?;
        #[cfg(feature="git")]
        fw.print_section(Field::Git)?;
        if fw.has_errors() {
            fw.print_line()?;
            fw.print_errors()?;
            fw.print_line()?;
        }
        fw.print_section(Field::Prompt)?;
        out_written.len()
    };
    std::io::stdout().write_all(&out[..out_len])?;
    Ok(())
}

fn main() -> Result<()> {
    let rval = std::env::args_os().nth(1).filter(|s|!s.is_empty()).map(|s|i32::from_str(&s.to_string_lossy())).transpose()?;
    print_default(rval)
}

// Not comprehensive, but sanity checking
#[cfg(test)]
mod test {
    use super::*;

    macro_rules! test {
        ($name:ident, $field:expr) => {
            #[test]
            fn $name() {
                {
                    let stdout = std::io::stdout();
                    setup(stdout.lock(), Some(0)).print_section($field).unwrap();
                }
                {
                    let stdout = std::io::stdout();
                    setup(stdout.lock(), Some(1)).print_section($field).unwrap();
                }
            }
        }
    }

    fn setup<T: Write>(stream: T, rval: Option<i32>) -> FieldWriter<T> {
        FieldWriter::new(stream, rval.into())
    }

    test!(exit_code, Field::ExitCode);
    #[cfg(feature="git")]
    test!(git, Field::Git);
    #[cfg(feature="network")]
    test!(network, Field::Network);
    #[cfg(feature="platform")]
    test!(platform, Field::Platform);
    test!(ppid, Field::Ppid);
    test!(prompt, Field::Prompt);
    test!(pwd, Field::Pwd);
    test!(time, Field::Time);
    #[cfg(feature="tty")]
    test!(tty, Field::Tty);
    test!(whoami, Field::Whoami);

    #[test]
    fn default() {
        print_default(Some(0)).unwrap();
        print_default(Some(1)).unwrap();
    }
}

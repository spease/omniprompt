use bytesize::ByteSize;
use chrono::Local;
use anyhow::{Context, Result, anyhow};
#[cfg(feature="git")]
use git2::Repository;
use std::ffi::{CStr, OsStr, OsString};
use std::io::Write;
use std::net::IpAddr;
use std::path::Path;
use std::str::FromStr;
use sysinfo::{NetworkExt, ProcessExt, RefreshKind, SystemExt};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, StandardStreamLock, WriteColor};

struct ColorCache {
    blue: ColorSpec,
    blue_bold: ColorSpec,
    cyan: ColorSpec,
    cyan_bold: ColorSpec,
    green_bold: ColorSpec,
    magenta: ColorSpec,
    magenta_bold: ColorSpec,
    red: ColorSpec,
    red_bold: ColorSpec,
    yellow: ColorSpec,
    yellow_bold: ColorSpec,
}

impl Default for ColorCache {
    fn default() -> Self {
        let mut out = ColorCache {
            blue: ColorSpec::new(),
            blue_bold: ColorSpec::new(),
            cyan: ColorSpec::new(),
            cyan_bold: ColorSpec::new(),
            green_bold: ColorSpec::new(),
            magenta: ColorSpec::new(),
            magenta_bold: ColorSpec::new(),
            red: ColorSpec::new(),
            red_bold: ColorSpec::new(),
            yellow: ColorSpec::new(),
            yellow_bold: ColorSpec::new(),
        };
        out.blue.set_fg(Some(Color::Blue));
        out.blue_bold.set_fg(Some(Color::Blue)).set_bold(true);
        out.cyan.set_fg(Some(Color::Cyan));
        out.cyan_bold.set_fg(Some(Color::Cyan)).set_bold(true);
        out.green_bold.set_fg(Some(Color::Green)).set_bold(true);
        out.magenta.set_fg(Some(Color::Magenta));
        out.magenta_bold.set_fg(Some(Color::Magenta)).set_bold(true);
        out.red.set_fg(Some(Color::Red));
        out.red_bold.set_fg(Some(Color::Red)).set_bold(true);
        out.yellow.set_fg(Some(Color::Yellow));
        out.yellow_bold.set_fg(Some(Color::Yellow)).set_bold(true);
        out
    }
}

struct FieldWriter {
    color_cache: ColorCache,
    column_count: usize,
    errors: String,
    exit_code: OsString,
    row_count: usize,
    stream: StandardStream,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum Field {
    ExitCode,
    #[cfg(feature="git")]
    Git,
    Network,
    Platform,
    Ppid,
    Prompt,
    Pwd,
    Time,
    #[cfg(unix)]
    Tty,
    Whoami,
}

impl Drop for FieldWriter {
    fn drop(&mut self) {
        self.stream.reset().expect("Failed to reset output stream");
    }
}

impl FieldWriter {
    fn new(stream: StandardStream, exit_code: OsString) -> Self {
        FieldWriter {
            color_cache: ColorCache::default(),
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

    fn print_field(function: Field, color_cache: &ColorCache, exit_code: &OsStr, stream: &mut StandardStreamLock) -> Result<()> {
        match function {
            Field::ExitCode => {
                stream.set_color(if exit_code == "0" { &color_cache.green_bold } else { &color_cache.red_bold})?;
                stream.write_all(exit_code.to_str().ok_or_else(||anyhow!("Unable to convert exit_code to string"))?.as_bytes())?;
            }
            #[cfg(feature="git")]
            Field::Git => {
                stream.set_color(&color_cache.yellow)?;
                if let Ok(repo) = Repository::discover(".") {
                    stream.write_all(repo.head().context("trying to get HEAD")?.shorthand().unwrap_or("<UNKNOWN>").as_bytes())?;
                }
            },
            Field::Network => {
                let si = sysinfo::System::new_with_specifics(RefreshKind::new().with_networks());
                let (upload, download) = si.networks().into_iter().map(|(_, nw)| (ByteSize(nw.received()), ByteSize(nw.transmitted()))).fold((ByteSize(0),ByteSize(0)), |sum,current|(sum.0+current.0, sum.1+current.1));
                write!(stream, "↑{}↓{}", upload, download)?;
            },
            Field::Platform => {
                stream.set_color(&color_cache.red)?;
                let arch = target_info::Target::arch();
                let oi = os_info::get();
                #[cfg(unix)]
                write!(
                    stream,
                    "{} ({})/{}/{}",
                    oi.os_type(),
                    oi.version(),
                    nix::sys::utsname::uname().release(),
                    arch
                )?;
                #[cfg(not(unix))]
                write!(stream, "{} ({})/{}", oi.os_type(), oi.version(), arch)?;
            },
            Field::Ppid => {
                stream.set_color(&color_cache.yellow)?;
                let si = sysinfo::System::new_with_specifics(RefreshKind::new().with_processes());
                let pid = sysinfo::get_current_pid().map_err(|e|anyhow!("{}",e))?;
                let parent_pid = si.process(pid).ok_or_else(||anyhow!("Couldn't find current PID"))?.parent().ok_or_else(||anyhow!("No parent for current process"))?;
                write!(stream, "{}", parent_pid)?;
            }
            Field::Prompt => {
                stream.set_color(&color_cache.magenta_bold)?;
                stream.write_all(b"$")?;
            }
            Field::Pwd => {
                stream.set_color(&color_cache.yellow_bold)?;
                let cwd = std::env::current_dir()?;
                let final_path = match dirs::home_dir() {
                    Some(home_dir) => match cwd.strip_prefix(home_dir) {
                        Ok(relpath) => Path::new("~").join(relpath),
                        Err(_) => cwd,
                    },
                    None => cwd,
                };
                write!(stream, "{}", final_path.display())?;
            }
            Field::Time => {
                stream.set_color(&color_cache.magenta)?;
                // stream.write_all(Local::now().to_rfc3339().as_bytes())?;
                write!(stream, "{}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f %Z"))?;
            }
            #[cfg(unix)]
            Field::Tty => {
                use std::os::unix::io::AsRawFd;
                stream.set_color(&color_cache.yellow)?;
                let stdin_fd = std::io::stdin().as_raw_fd();
                let tty_name = unsafe {
                    let tty_name_ptr = libc::ttyname(stdin_fd);
                    if !tty_name_ptr.is_null() {
                        CStr::from_ptr(tty_name_ptr).to_str()?
                    } else {
                        ""
                    }
                };
                stream.write_all(tty_name.as_bytes())?;
            }
            Field::Whoami => {
                stream.set_color(&color_cache.cyan_bold)?;
                stream.write_all(whoami::username().as_bytes())?;
                stream.set_color(&color_cache.cyan)?;
                stream.write_all(b"@")?;
                stream.set_color(&color_cache.cyan_bold)?;
                stream.write_all(whoami::hostname().as_bytes())?;
                if let Some(ssh_connection) = std::env::var_os("SSH_CONNECTION") {
                    let mut pieces = ssh_connection.to_str().ok_or_else(||anyhow!("Invalid UTF-8 for SSH_CONNECTION"))?.split(' ').skip(2);
                    let ssh_server_ip = IpAddr::from_str(pieces.next().ok_or_else(||anyhow!("Missing server IP"))?)?;
                    let ssh_server_port = u16::from_str(pieces.next().ok_or_else(||anyhow!("Missing server port"))?)?;

                    stream.set_color(&color_cache.cyan)?;
                    write!(stream, " ({}:{})", ssh_server_ip, ssh_server_port)?;
                }
            }
        }

        Ok(())
    }

    fn print_section(&mut self, function: Field) -> Result<()> {
        let mut stream = self.stream.lock();

        if self.column_count != 0 {
            stream.reset()?;
            stream.write_all(if self.row_count == 0 { b" - " } else { b"-" })?;
        }
        stream.set_color(&self.color_cache.blue_bold)?;
        stream.write_all(if self.column_count != 0 { b"[" } else if self.row_count == 0 { "┌─[".as_bytes() } else { "└─[".as_bytes() })?;

        if let Err(e) = Self::print_field(function, &self.color_cache, &self.exit_code, &mut stream) {
            use std::fmt::Write;
            if self.errors.is_empty() {
                write!(self.errors, "{:?}", e)?;
            } else {
                write!(self.errors, "\n{:?}", e)?;
            }
        }
        self.column_count += 1;

        stream.set_color(&self.color_cache.blue_bold)?;
        stream.write_all(if function != Field::Prompt { b"]" } else { b"]> " })?;

        Ok(())
    }

    fn print_errors(&mut self) -> Result<()> {
        let mut stream = self.stream.lock();
        stream.set_color(&self.color_cache.red_bold)?;
        stream.write_all(self.errors.as_bytes())?;
        Ok(())
    }

    fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

fn print_default(rval: impl Into<OsString>) -> Result<()> {
    let mut fw = FieldWriter::new(StandardStream::stdout(ColorChoice::Always), rval.into());

    fw.print_section(Field::Whoami)?;
    fw.print_section(Field::Pwd)?;
    #[cfg(unix)]
    fw.print_section(Field::Ppid)?;
    fw.print_section(Field::Time)?;
    fw.print_section(Field::Platform)?;
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
    Ok(())
}

fn main() -> Result<()> {
    let rval = std::env::args_os()
        .nth(1)
        .unwrap_or_default();
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
                setup("0").print_section($field).unwrap();
                setup("E").print_section($field).unwrap();
            }
        }
    }

    fn setup(rval: impl Into<OsString>) -> FieldWriter {
        FieldWriter::new(StandardStream::stdout(ColorChoice::Always), rval.into())
    }

    test!(exit_code, Field::ExitCode);
    #[cfg(feature="git")]
    test!(git, Field::Git);
    test!(network, Field::Network);
    test!(platform, Field::Platform);
    test!(ppid, Field::Ppid);
    test!(prompt, Field::Prompt);
    test!(pwd, Field::Pwd);
    test!(time, Field::Time);
    #[cfg(unix)]
    test!(tty, Field::Tty);
    test!(whoami, Field::Whoami);

    #[test]
    fn default() {
        print_default("0").unwrap();
        print_default("E").unwrap();
    }
}

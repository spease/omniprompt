use bytesize::ByteSize;
use chrono::Local;
use failure::{Error, ResultExt, format_err};
use no_panic::no_panic;
#[cfg(feature="git")]
use git2::Repository;
use std::ffi::{CStr, OsString};
use std::io::Write;
use std::net::IpAddr;
use std::path::Path;
use std::str::FromStr;
use sysinfo::{NetworkExt, ProcessExt, SystemExt};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

type Result<T> = ::std::result::Result<T, Error>;

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
            blue: ColorSpec::new().set_fg(Some(Color::Blue)).clone(),
            blue_bold: ColorSpec::new()
                .set_fg(Some(Color::Blue))
                .set_bold(true)
                .clone(),
            cyan: ColorSpec::new().set_fg(Some(Color::Cyan)).clone(),
            cyan_bold: ColorSpec::new()
                .set_fg(Some(Color::Cyan))
                .set_bold(true)
                .clone(),
            green_bold: ColorSpec::new()
                .set_fg(Some(Color::Green))
                .set_bold(true)
                .clone(),
            magenta: ColorSpec::new().set_fg(Some(Color::Magenta)).clone(),
            magenta_bold: ColorSpec::new()
                .set_fg(Some(Color::Magenta))
                .set_bold(true)
                .clone(),
            red: ColorSpec::new().set_fg(Some(Color::Red)).clone(),
            red_bold: ColorSpec::new()
                .set_fg(Some(Color::Red))
                .set_bold(true)
                .clone(),
            yellow: ColorSpec::new().set_fg(Some(Color::Yellow)).clone(),
            yellow_bold: ColorSpec::new()
                .set_fg(Some(Color::Yellow))
                .set_bold(true)
                .clone(),
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
    exit_code: OsString,
    row_count: usize,
    stream: StandardStream,
}

#[derive(Eq, PartialEq)]
enum Function {
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
    #[no_panic]
    fn new(stream: StandardStream, exit_code: OsString) -> Self {
        FieldWriter {
            color_cache: ColorCache::default(),
            column_count: 0,
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

    fn print_section(&mut self, function: Function) -> Result<()> {
        let mut stream = self.stream.lock();

        if self.column_count != 0 {
            stream.reset()?;
            stream.write_all(if self.row_count == 0 { b" - " } else { b"-" })?;
        }
        stream.set_color(&self.color_cache.blue_bold)?;
        stream.write_all(if self.column_count != 0 { b"[" } else if self.row_count == 0 { "┌─[".as_bytes() } else { "└─[".as_bytes() })?;

        match function {
            Function::ExitCode => {
                if self.exit_code == "0" {
                    stream.set_color(&self.color_cache.green_bold)?;
                } else {
                    stream.set_color(&self.color_cache.red_bold)?;
                }
                stream.write_all(self.exit_code.to_str().ok_or_else(||format_err!("Unable to convert exit_code to string"))?.as_bytes())?;
            }
            #[cfg(feature="git")]
            Function::Git => {
                stream.set_color(&self.color_cache.yellow)?;
                let repo = Repository::discover(".").context("trying to find git repo")?;
                let head = repo.head().context("trying to get HEAD")?;

                stream.write_all(head.shorthand().unwrap_or("").as_bytes())?;
            },
            Function::Network => {
                let si = sysinfo::System::new();
                let nw = si.get_network();
                let upload = ByteSize(nw.get_income());
                let download = ByteSize(nw.get_outcome());
                write!(stream, "↑{}↓{}", upload, download)?;
            },
            Function::Platform => {
                stream.set_color(&self.color_cache.red)?;
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
            Function::Ppid => {
                stream.set_color(&self.color_cache.yellow)?;
                let pid = sysinfo::get_current_pid().map_err(|e|format_err!("{}",e))?;
                let si = sysinfo::System::new();
                let parent_pid = si.get_process(pid).ok_or_else(||format_err!("Couldn't find current PID"))?.parent().ok_or_else(||format_err!("No parent for current process"))?;
                write!(stream, "{}", parent_pid)?;
            }
            Function::Prompt => {
                stream.set_color(&self.color_cache.magenta_bold)?;
                stream.write_all(b"$")?;
            }
            Function::Pwd => {
                stream.set_color(&self.color_cache.yellow_bold)?;
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
            Function::Time => {
                stream.set_color(&self.color_cache.magenta)?;
                // stream.write_all(Local::now().to_rfc3339().as_bytes())?;
                write!(stream, "{}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f %Z"))?;
            }
            #[cfg(unix)]
            Function::Tty => {
                use std::os::unix::io::AsRawFd;
                stream.set_color(&self.color_cache.yellow)?;
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
            Function::Whoami => {
                stream.set_color(&self.color_cache.cyan_bold)?;
                stream.write_all(whoami::username().as_bytes())?;
                stream.set_color(&self.color_cache.cyan)?;
                stream.write_all(b"@")?;
                stream.set_color(&self.color_cache.cyan_bold)?;
                stream.write_all(whoami::hostname().as_bytes())?;
                if let Some(ssh_connection) = std::env::var_os("SSH_CONNECTION") {
                    let mut pieces = ssh_connection.to_str().ok_or_else(||format_err!("Invalid UTF-8 for SSH_CONNECTION"))?.split(' ').skip(2);
                    let ssh_server_ip = IpAddr::from_str(pieces.next().ok_or_else(||format_err!("Missing server IP"))?)?;
                    let ssh_server_port = u16::from_str(pieces.next().ok_or_else(||format_err!("Missing server port"))?)?;

                    stream.set_color(&self.color_cache.cyan)?;
                    write!(stream, " ({}:{})", ssh_server_ip, ssh_server_port)?;
                }
            }
        }

        self.column_count += 1;

        stream.set_color(&self.color_cache.blue_bold)?;
        stream.write_all(if function != Function::Prompt { b"]" } else { b"]> " })?;

        Ok(())
    }
}

fn main() -> Result<()> {
    let rval = std::env::args_os()
        .into_iter()
        .skip(1)
        .next()
        .unwrap_or(OsString::new());
    let mut fw = FieldWriter::new(StandardStream::stdout(ColorChoice::Always), rval);

    fw.print_section(Function::Whoami)?;
    fw.print_section(Function::Pwd)?;
    #[cfg(unix)]
    fw.print_section(Function::Ppid)?;
    fw.print_section(Function::Time)?;
    fw.print_section(Function::Platform)?;
    fw.print_line()?;
    fw.print_section(Function::ExitCode)?;
    #[cfg(feature="git")]
    fw.print_section(Function::Git)?;
    fw.print_section(Function::Prompt)?;
    Ok(())
}

//! A stand-in for MariaDB's client tools, for the backups' tests. Copied as `mariadb-dump` it
//! writes a small dump of the database it is given; copied as `mariadb` it answers the connection
//! test. Either way it reads the connection from the option file its first argument names, as the
//! real tools do, and refuses to run when the password also reached its command line.
//!
//! The database's name says how it behaves: `fail` refuses the login, `cut` stops before the last
//! line, `slow<ms>` takes that long, `loose` adds a table whose engine has no transactions.

use std::io::{self, Write};
use std::process;
use std::thread;
use std::time::{Duration, Instant};

const VERSION: &str = "11.7.2-MariaDB";

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let program = std::env::current_exe()
        .ok()
        .and_then(|path| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let dump = program.contains("dump");
    if arguments.first().map(String::as_str) == Some("--version") {
        let name = if dump { "mariadb-dump" } else { "mariadb" };
        println!("{name} from {VERSION}, client 10.19 for Win64 (AMD64)");
        return;
    }
    let options = arguments
        .first()
        .and_then(|first| first.strip_prefix("--defaults-file="))
        .unwrap_or_else(|| fail(2, "the first argument must be --defaults-file"));
    let text =
        std::fs::read_to_string(options).unwrap_or_else(|error| fail(2, &format!("{options}: {error}")));
    let value = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .map(|value| {
                value
                    .trim_matches('"')
                    .replace("\\\"", "\"")
                    .replace("\\\\", "\\")
            })
            .unwrap_or_default()
    };
    let password = value("password");
    if !password.is_empty() && arguments.iter().any(|argument| argument.contains(&password)) {
        fail(3, "the password reached the command line");
    }
    let database = arguments.last().cloned().unwrap_or_default();
    let user = value("user");
    if database.contains("fail") {
        fail(
            2,
            &format!(
                "mariadb-dump: Got error: 1045: \"Access denied for user '{user}'@'localhost' (using password: YES)\" \
                 when trying to connect"
            ),
        );
    }
    if !dump {
        println!("{VERSION}\t2\t32768\t{}", u32::from(database.contains("loose")));
        return;
    }
    let host = value("host");
    let port = value("port");
    let mut out = io::stdout().lock();
    let _ = write!(
        out,
        "/*M!999999\\- enable the sandbox mode */ \n-- MariaDB dump 10.19-{VERSION}, for Win64 (AMD64)\n--\n\
         -- Host: {host}:{port}    Database: {database}\n-- User: {user}\n\
         CREATE TABLE `users` (\n  `identifier` varchar(60) NOT NULL\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;\n"
    );
    if database.contains("loose") {
        let _ = write!(
            out,
            "CREATE TABLE `player_logs` (\n  `id` int(11) NOT NULL\n) ENGINE=Aria DEFAULT CHARSET=utf8mb4;\n"
        );
    }
    let slow = database
        .split("slow")
        .nth(1)
        .and_then(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .map(Duration::from_millis);
    let started = Instant::now();
    let mut row = 0;
    loop {
        let _ = writeln!(out, "INSERT INTO `users` VALUES ('license:{row:040}');");
        row += 1;
        match slow {
            Some(slow) if started.elapsed() < slow => {
                let _ = out.flush();
                thread::sleep(Duration::from_millis(20));
            }
            _ if row < 1000 => {}
            _ => break,
        }
    }
    if !database.contains("cut") {
        let _ = writeln!(out, "-- Dump completed on 2026-11-02  4:00:01");
    }
    let _ = out.flush();
}

fn fail(code: i32, message: &str) -> ! {
    eprintln!("{message}");
    process::exit(code)
}

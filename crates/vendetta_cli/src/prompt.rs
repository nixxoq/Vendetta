use std::io::{self, BufRead, IsTerminal, Write};

pub fn prompt_line(prompt: &str) -> io::Result<String> {
    eprint!("{prompt}");
    io::stderr().flush()?;
    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

pub fn prompt_secret(prompt: &str) -> io::Result<String> {
    eprint!("{prompt}");
    io::stderr().flush()?;
    let secret = rpassword::read_password()?;
    Ok(secret.trim().to_string())
}

pub fn prompt_confirm(prompt: &str, default: bool) -> io::Result<bool> {
    if !std::io::stdin().is_terminal() {
        return Ok(default);
    }
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    let input = prompt_line(&format!("{prompt} {hint} "))?;
    let input_lower = input.to_lowercase();
    if input_lower.is_empty() {
        Ok(default)
    } else {
        Ok(input_lower == "y" || input_lower == "yes")
    }
}

use crate::{
    error::{ErrorKind, Result, SearvornError},
    task::CancellationFlag,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    program: String,
    args: Vec<String>,
    cwd: Option<String>,
    env: Vec<(String, String)>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>) -> Result<Self> {
        let program = program.into();
        validate_field(&program, "execution.program")?;

        Ok(Self {
            program,
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        })
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Result<Self> {
        let arg = arg.into();
        validate_nul_free(&arg, "execution.arg")?;
        self.args.push(arg);
        Ok(self)
    }

    pub fn cwd(mut self, cwd: impl Into<String>) -> Result<Self> {
        let cwd = cwd.into();
        validate_field(&cwd, "execution.cwd")?;
        self.cwd = Some(cwd);
        Ok(self)
    }

    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let value = value.into();

        validate_field(&name, "execution.env")?;
        validate_nul_free(&value, "execution.env")?;
        if name.contains('=') {
            return Err(SearvornError::new(
                ErrorKind::InvalidInput,
                "execution.env",
            ));
        }

        self.env.push((name, value));
        Ok(self)
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn cwd_path(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    pub fn environment(&self) -> &[(String, String)] {
        &self.env
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionLimits {
    pub max_stdout_bytes: u64,
    pub max_stderr_bytes: u64,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            max_stdout_bytes: 4 * 1024 * 1024,
            max_stderr_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionResult {
    pub exit_code: Option<i32>,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

pub trait ExecutionObserver {
    fn stdout(&mut self, chunk: &[u8]) -> Result<()>;
    fn stderr(&mut self, chunk: &[u8]) -> Result<()>;
}

pub trait ExecutionBackend: Send + Sync {
    fn name(&self) -> &'static str;

    fn execute(
        &self,
        command: &CommandSpec,
        limits: ExecutionLimits,
        cancellation: Option<&CancellationFlag>,
        observer: &mut dyn ExecutionObserver,
    ) -> Result<ExecutionResult>;
}

#[derive(Debug)]
pub struct BoundedOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    limits: ExecutionLimits,
    stdout_seen: u64,
    stderr_seen: u64,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

impl BoundedOutput {
    pub fn new(limits: ExecutionLimits) -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
            limits,
            stdout_seen: 0,
            stderr_seen: 0,
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    pub fn stdout_bytes(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stderr_bytes(&self) -> &[u8] {
        &self.stderr
    }

    pub const fn stdout_seen(&self) -> u64 {
        self.stdout_seen
    }

    pub const fn stderr_seen(&self) -> u64 {
        self.stderr_seen
    }

    pub const fn stdout_truncated(&self) -> bool {
        self.stdout_truncated
    }

    pub const fn stderr_truncated(&self) -> bool {
        self.stderr_truncated
    }

    fn append(
        target: &mut Vec<u8>,
        seen: &mut u64,
        truncated: &mut bool,
        limit: u64,
        chunk: &[u8],
    ) {
        *seen = seen.saturating_add(chunk.len() as u64);

        let remaining = limit.saturating_sub(target.len() as u64);
        let copy = chunk.len().min(remaining.min(usize::MAX as u64) as usize);
        target.extend_from_slice(&chunk[..copy]);

        if copy < chunk.len() {
            *truncated = true;
        }
    }
}

impl ExecutionObserver for BoundedOutput {
    fn stdout(&mut self, chunk: &[u8]) -> Result<()> {
        Self::append(
            &mut self.stdout,
            &mut self.stdout_seen,
            &mut self.stdout_truncated,
            self.limits.max_stdout_bytes,
            chunk,
        );
        Ok(())
    }

    fn stderr(&mut self, chunk: &[u8]) -> Result<()> {
        Self::append(
            &mut self.stderr,
            &mut self.stderr_seen,
            &mut self.stderr_truncated,
            self.limits.max_stderr_bytes,
            chunk,
        );
        Ok(())
    }
}

fn validate_field(value: &str, operation: &'static str) -> Result<()> {
    if value.is_empty() {
        return Err(SearvornError::new(ErrorKind::InvalidInput, operation));
    }

    validate_nul_free(value, operation)
}

fn validate_nul_free(value: &str, operation: &'static str) -> Result<()> {
    if value.as_bytes().contains(&0) {
        Err(SearvornError::new(ErrorKind::InvalidInput, operation))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{BoundedOutput, CommandSpec, ExecutionLimits, ExecutionObserver};
    use crate::ErrorKind;

    #[test]
    fn command_spec_keeps_argv_structured() {
        let command = CommandSpec::new("/system/bin/echo")
            .expect("command")
            .arg("a b")
            .expect("arg")
            .arg("$HOME")
            .expect("arg")
            .cwd("/data/local/tmp")
            .expect("cwd")
            .env("MODE", "safe")
            .expect("env");

        assert_eq!(command.program(), "/system/bin/echo");
        assert_eq!(command.args(), ["a b", "$HOME"]);
        assert_eq!(command.cwd_path(), Some("/data/local/tmp"));
        assert_eq!(command.environment(), [("MODE".to_owned(), "safe".to_owned())]);
    }

    #[test]
    fn rejects_ambiguous_environment_names_and_nuls() {
        assert_eq!(
            CommandSpec::new("x")
                .expect("command")
                .env("A=B", "x")
                .expect_err("invalid env")
                .kind(),
            ErrorKind::InvalidInput
        );

        assert_eq!(
            CommandSpec::new("bad\0program")
                .expect_err("nul program")
                .kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn bounded_output_counts_but_does_not_retain_over_limit() {
        let mut output = BoundedOutput::new(ExecutionLimits {
            max_stdout_bytes: 4,
            max_stderr_bytes: 2,
        });

        output.stdout(b"abcdef").expect("stdout");
        output.stderr(b"xyz").expect("stderr");

        assert_eq!(output.stdout_bytes(), b"abcd");
        assert_eq!(output.stderr_bytes(), b"xy");
        assert_eq!(output.stdout_seen(), 6);
        assert_eq!(output.stderr_seen(), 3);
        assert!(output.stdout_truncated());
        assert!(output.stderr_truncated());
    }
}

//! Execução controlada do Git.
//!
//! Regras:
//! - nunca passa por shell: o executável é chamado diretamente, com argumentos separados;
//! - o ambiente é endurecido para que a execução seja previsível (as operações de escrita
//!   ficam em módulos próprios e só executam subcomandos predefinidos);
//! - toda execução tem tempo limite e limite de saída;
//! - falhas são classificadas em erros compreensíveis (`GitError`).

use crate::error::{ErrorCode, GitError, Result};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

/// Caminho do Git já validado (só guarda sucesso: instalar o Git depois funciona sem reiniciar).
static LOCATED: Mutex<Option<PathBuf>> = Mutex::new(None);

const MAX_STDERR_BYTES: usize = 64 * 1024;

/// Opções globais aplicadas a TODA chamada:
/// - `--no-pager` / `--no-optional-locks`: sem paginação e sem gravar o índice ao consultar;
/// - `core.fsmonitor=false`: impede que a configuração do repositório execute um comando externo;
/// - `core.quotepath=off` / `color.ui=false`: saída previsível.
const GLOBAL_ARGS: &[&str] = &[
    "--no-pager",
    "--no-optional-locks",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.quotepath=off",
    "-c",
    "color.ui=false",
];

/// Variáveis de ambiente herdadas que poderiam redirecionar ou alterar o comportamento do Git.
const SCRUBBED_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_EXTERNAL_DIFF",
    "GIT_PAGER",
    "GIT_CONFIG_PARAMETERS",
];

#[derive(Debug, Clone)]
pub struct RunOptions<'a> {
    pub timeout: Duration,
    pub max_stdout: usize,
    /// Códigos de saída considerados sucesso (ex.: `git diff --no-index` retorna 1 quando há diferenças).
    pub ok_exit_codes: &'a [i32],
}

impl Default for RunOptions<'_> {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_stdout: 4 * 1024 * 1024,
            ok_exit_codes: &[0],
        }
    }
}

#[derive(Debug)]
pub struct GitOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    /// `true` se a saída passou de `max_stdout` e foi cortada.
    pub stdout_truncated: bool,
}

/// Resultado bruto de uma execução: o código de saída não foi interpretado.
#[derive(Debug)]
pub struct RawOutput {
    /// `None` quando o processo foi encerrado por um sinal.
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// `true` se a saída passou de `max_stdout` e foi cortada.
    pub stdout_truncated: bool,
}

/// Referência a um executável do Git.
#[derive(Debug, Clone)]
pub struct Git {
    program: PathBuf,
}

impl Git {
    /// Localiza o Git instalado (PATH e, no Windows, locais de instalação comuns).
    pub fn locate() -> Result<Git> {
        if let Some(program) = LOCATED.lock().ok().and_then(|guard| guard.clone()) {
            return Ok(Git { program });
        }

        let mut last_error: Option<GitError> = None;
        for candidate in candidates() {
            let git = Git { program: candidate };
            match git.version() {
                Ok(_) => {
                    if let Ok(mut guard) = LOCATED.lock() {
                        *guard = Some(git.program.clone());
                    }
                    return Ok(git);
                }
                Err(err) => last_error = Some(err),
            }
        }

        // Se algum candidato existia mas falhou ao executar, preserva esse diagnóstico.
        match last_error {
            Some(err) if err.code != ErrorCode::GitNotFound => Err(err),
            _ => Err(git_not_found()),
        }
    }

    /// Usa um executável específico (útil para testes e para configuração futura).
    pub fn with_program(program: impl Into<PathBuf>) -> Git {
        Git {
            program: program.into(),
        }
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Retorna a versão (ex.: `2.43.0.windows.1`).
    pub fn version(&self) -> Result<String> {
        let opts = RunOptions {
            timeout: Duration::from_secs(10),
            max_stdout: 4096,
            ..RunOptions::default()
        };
        let out = self.run(None, &["--version"], &opts)?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().next().unwrap_or("").trim();
        let version = line.strip_prefix("git version ").unwrap_or(line).trim();
        if version.is_empty() {
            return Err(GitError::new(
                ErrorCode::UnexpectedOutput,
                "O Git respondeu, mas não informou a versão.",
            ));
        }
        Ok(version.to_string())
    }

    /// Executa `git <args>` (sem shell) e aplica as proteções descritas no módulo.
    /// Códigos de saída fora de `opts.ok_exit_codes` viram um erro classificado.
    pub fn run(&self, cwd: Option<&Path>, args: &[&str], opts: &RunOptions) -> Result<GitOutput> {
        let raw = self.run_raw(cwd, args, None, opts)?;
        match raw.exit_code {
            Some(code) if opts.ok_exit_codes.contains(&code) => Ok(GitOutput {
                exit_code: code,
                stdout: raw.stdout,
                stdout_truncated: raw.stdout_truncated,
            }),
            code => Err(classify_failure(code, &String::from_utf8_lossy(&raw.stderr))),
        }
    }

    /// Como `run`, mas devolve stdout, stderr e o código de saída **sem interpretá-los**
    /// (`opts.ok_exit_codes` é ignorado). Permite que as operações de escrita expliquem a falha
    /// com precisão. `stdin` (opcional) é enviado ao processo, por exemplo para listas de caminhos
    /// em `--pathspec-from-file=-` (evita o limite de tamanho da linha de comando do Windows).
    pub fn run_raw(
        &self,
        cwd: Option<&Path>,
        args: &[&str],
        stdin: Option<&[u8]>,
        opts: &RunOptions,
    ) -> Result<RawOutput> {
        if let Some(dir) = cwd {
            if !dir.is_dir() {
                return Err(GitError::new(
                    ErrorCode::PathNotFound,
                    "A pasta do repositório não existe mais ou não está acessível.",
                ));
            }
        }

        let mut cmd = Command::new(&self.program);
        cmd.args(GLOBAL_ARGS)
            .args(args)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("LC_ALL", "C")
            .env("LANGUAGE", "C");
        for var in SCRUBBED_ENV {
            cmd.env_remove(var);
        }
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        hide_console_window(&mut cmd);

        let mut child = cmd.spawn().map_err(|err| self.spawn_error(&err))?;

        // A entrada é escrita em outra thread para não travar caso o Git encha o pipe de saída.
        let stdin_pipe = child.stdin.take();
        let input: Option<Vec<u8>> = stdin.map(|bytes| bytes.to_vec());
        let writer = thread::spawn(move || {
            if let (Some(mut pipe), Some(data)) = (stdin_pipe, input) {
                // Erros aqui (ex.: o Git encerrou antes de ler tudo) aparecem no código de saída.
                let _ = pipe.write_all(&data);
            }
            // `pipe` é descartado aqui, o que fecha a entrada do processo.
        });

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let max_stdout = opts.max_stdout;
        let out_reader = thread::spawn(move || read_capped(stdout, max_stdout));
        let err_reader = thread::spawn(move || read_capped(stderr, MAX_STDERR_BYTES));

        let deadline = Instant::now() + opts.timeout;
        let mut pause = Duration::from_millis(1);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        // Os leitores terminam quando os pipes fecham; não bloqueamos esperando por eles.
                        return Err(GitError::new(
                            ErrorCode::Timeout,
                            format!(
                                "O Git demorou mais de {} s para responder e a operação foi cancelada.",
                                opts.timeout.as_secs().max(1)
                            ),
                        ));
                    }
                    thread::sleep(pause);
                    pause = (pause * 2).min(Duration::from_millis(20));
                }
                Err(err) => {
                    let _ = child.kill();
                    return Err(GitError::new(
                        ErrorCode::GitUnavailable,
                        "Não foi possível acompanhar a execução do Git.",
                    )
                    .with_details(err.to_string()));
                }
            }
        };

        let _ = writer.join();
        let (stdout, stdout_truncated) = out_reader.join().unwrap_or_default();
        let (stderr, _) = err_reader.join().unwrap_or_default();

        Ok(RawOutput {
            exit_code: status.code(),
            stdout,
            stderr,
            stdout_truncated,
        })
    }

    fn spawn_error(&self, err: &std::io::Error) -> GitError {
        use std::io::ErrorKind;
        match err.kind() {
            ErrorKind::NotFound => {
                // O caminho em cache deixou de existir (Git desinstalado/movido): esquece-o.
                if let Ok(mut guard) = LOCATED.lock() {
                    if guard.as_deref() == Some(self.program.as_path()) {
                        *guard = None;
                    }
                }
                git_not_found()
            }
            ErrorKind::PermissionDenied => GitError::new(
                ErrorCode::GitUnavailable,
                "O Git foi encontrado, mas o sistema não permitiu executá-lo.",
            )
            .with_details(err.to_string()),
            _ => GitError::new(
                ErrorCode::GitUnavailable,
                "Não foi possível executar o Git.",
            )
            .with_details(err.to_string()),
        }
    }
}

fn git_not_found() -> GitError {
    GitError::new(
        ErrorCode::GitNotFound,
        "O Git não foi encontrado neste computador. Instale o Git for Windows \
         (https://git-scm.com/download/win), confirme que ele está disponível no PATH e reinicie o aplicativo.",
    )
}

/// Traduz uma falha do Git em um erro compreensível.
fn classify_failure(code: Option<i32>, stderr: &str) -> GitError {
    let lower = stderr.to_lowercase();
    if lower.contains("not a git repository") {
        return GitError::new(
            ErrorCode::NotARepository,
            "A pasta selecionada não é um repositório Git.",
        );
    }
    if lower.contains("must be run in a work tree") || lower.contains("this operation must be run in a work tree") {
        return GitError::new(
            ErrorCode::BareRepository,
            "Esta pasta não tem um diretório de trabalho (repositório bare ou pasta interna .git). \
             Selecione a pasta do projeto.",
        );
    }
    if lower.contains("dubious ownership") || lower.contains("safe.directory") {
        return GitError::new(
            ErrorCode::UnsafeRepository,
            "O Git recusou este repositório porque a pasta pertence a outro usuário (proteção safe.directory). \
             Abra o repositório com o usuário correto ou marque-o como seguro no Git.",
        )
        .with_details(stderr);
    }
    let message = match code {
        Some(c) => format!("O Git terminou com erro (código {c})."),
        None => "O Git foi interrompido antes de concluir.".to_string(),
    };
    GitError::new(ErrorCode::CommandFailed, message).with_details(stderr)
}

/// Lê até `cap` bytes e descarta o excedente (sem bloquear o processo filho).
fn read_capped<R: Read>(reader: Option<R>, cap: usize) -> (Vec<u8>, bool) {
    let Some(mut reader) = reader else {
        return (Vec::new(), false);
    };
    let mut data = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = cap.saturating_sub(data.len());
                if n > room {
                    data.extend_from_slice(&chunk[..room]);
                    truncated = true;
                } else {
                    data.extend_from_slice(&chunk[..n]);
                }
            }
        }
    }
    (data, truncated)
}

fn candidates() -> Vec<PathBuf> {
    let mut list = vec![PathBuf::from("git")];
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(base) = std::env::var_os(var) {
                list.push(Path::new(&base).join("Git").join("cmd").join("git.exe"));
            }
        }
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            list.push(
                Path::new(&base)
                    .join("Programs")
                    .join("Git")
                    .join("cmd")
                    .join("git.exe"),
            );
        }
        // Mantém "git" primeiro; os demais só entram se o arquivo existir.
        let mut seen = Vec::new();
        list.retain(|p| {
            let keep = p == Path::new("git") || (p.is_file() && !seen.contains(p));
            if keep {
                seen.push(p.clone());
            }
            keep
        });
    }
    list
}

/// Evita que uma janela de console pisque a cada chamada ao Git no Windows.
#[cfg(windows)]
fn hide_console_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console_window(_cmd: &mut Command) {}

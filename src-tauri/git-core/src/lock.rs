//! Exclusão mútua das operações de **escrita** por repositório.
//!
//! Impede que duas operações incompatíveis (por exemplo, dois commits ou um commit durante uma
//! troca de branch) rodem ao mesmo tempo sobre o mesmo repositório, mesmo que a interface
//! envie cliques repetidos. A trava é liberada automaticamente quando a operação termina.

use crate::error::{ErrorCode, GitError, Result};
use std::{path::Path, sync::Mutex};

static BUSY: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Garante a exclusividade enquanto existir. Ao ser descartado, libera o repositório.
#[derive(Debug)]
pub struct RepoGuard {
    key: String,
}

impl RepoGuard {
    pub fn acquire(root: &Path) -> Result<RepoGuard> {
        let mut key = root.to_string_lossy().into_owned();
        if cfg!(windows) {
            // O Windows não diferencia maiúsculas de minúsculas nos caminhos.
            key = key.to_lowercase();
        }
        let mut busy = BUSY.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if busy.contains(&key) {
            return Err(GitError::new(
                ErrorCode::RepositoryBusy,
                "Outra operação ainda está em andamento neste repositório. \
                 Aguarde ela terminar e tente novamente.",
            ));
        }
        busy.push(key.clone());
        Ok(RepoGuard { key })
    }
}

impl Drop for RepoGuard {
    fn drop(&mut self) {
        let mut busy = BUSY.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        busy.retain(|k| k != &self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn second_acquire_fails_until_first_is_dropped() {
        let root = PathBuf::from("/tmp/gitflow-lock-test-unico");
        let first = RepoGuard::acquire(&root).unwrap();
        let err = RepoGuard::acquire(&root).unwrap_err();
        assert_eq!(err.code, ErrorCode::RepositoryBusy);
        drop(first);
        assert!(RepoGuard::acquire(&root).is_ok());
    }

    #[test]
    fn different_repositories_do_not_block_each_other() {
        let a = RepoGuard::acquire(&PathBuf::from("/tmp/gitflow-lock-test-a")).unwrap();
        let b = RepoGuard::acquire(&PathBuf::from("/tmp/gitflow-lock-test-b"));
        assert!(b.is_ok());
        drop(a);
    }
}

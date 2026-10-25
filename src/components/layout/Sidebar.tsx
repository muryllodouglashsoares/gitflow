import type { ReactNode } from "react";
import type { GitState } from "../../hooks/useRepositoryWorkspace";
import { Button } from "../common/Button";
import { BranchIcon, ChangesIcon, FolderOpenIcon, LogoMark } from "../common/icons";
import { Spinner } from "../common/Spinner";

/**
 * Itens de navegação. As próximas fases acrescentam entradas aqui
 * (e uma tela correspondente em App.tsx).
 */
export type NavId = "changes" | "branches";

interface NavItem {
  id: NavId;
  label: string;
  icon: ReactNode;
}

const NAV_ITEMS: NavItem[] = [
  { id: "changes", label: "Alterações", icon: <ChangesIcon /> },
  { id: "branches", label: "Branches", icon: <BranchIcon /> },
];

interface SidebarProps {
  active: NavId;
  git: GitState;
  busy: boolean;
  /** Impede abrir outro repositório (ex.: operação de escrita em andamento). */
  openDisabled?: boolean;
  onOpenRepository: () => void;
  onNavigate: (id: NavId) => void;
}

export function Sidebar({ active, git, busy, openDisabled = false, onOpenRepository, onNavigate }: SidebarProps) {
  return (
    <aside className="flex w-56 shrink-0 flex-col border-r border-line bg-surface" aria-label="Navegação">
      <div className="flex items-center gap-2.5 px-4 pt-4 pb-3">
        <LogoMark />
        <span className="text-[15px] font-semibold tracking-tight">GitFlow Desktop</span>
      </div>

      <div className="px-3 pb-3">
        <Button variant="secondary" className="w-full" onClick={onOpenRepository} disabled={busy || openDisabled}>
          {busy ? <Spinner /> : <FolderOpenIcon />}
          {busy ? "Abrindo…" : "Abrir repositório"}
        </Button>
      </div>

      <nav className="flex-1 px-2" aria-label="Seções">
        <ul className="space-y-0.5">
          {NAV_ITEMS.map((item) => {
            const isActive = item.id === active;
            return (
              <li key={item.id}>
                <button
                  type="button"
                  onClick={() => onNavigate(item.id)}
                  aria-current={isActive ? "page" : undefined}
                  className={`flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-sm transition-colors ${
                    isActive ? "bg-raised text-fg" : "text-muted hover:bg-raised/60 hover:text-fg"
                  }`}
                >
                  <span className={isActive ? "text-accent" : ""}>{item.icon}</span>
                  {item.label}
                </button>
              </li>
            );
          })}
        </ul>
      </nav>

      <GitStatusFooter git={git} />
    </aside>
  );
}

function GitStatusFooter({ git }: { git: GitState }) {
  let dot = "bg-faint";
  let text = "Verificando Git…";
  let title: string | undefined;

  if (!git.checking && git.info) {
    if (git.info.available) {
      dot = "bg-added";
      text = `Git ${git.info.version ?? ""}`.trim();
      title = git.info.programPath ?? undefined;
    } else {
      dot = "bg-removed";
      text = "Git indisponível";
      title = git.info.error?.message;
    }
  }

  return (
    <div className="border-t border-line px-4 py-3 text-xs text-muted" title={title} aria-live="polite">
      <span className="flex items-center gap-2">
        <span className={`h-2 w-2 shrink-0 rounded-full ${dot}`} aria-hidden="true" />
        <span className="truncate">{text}</span>
      </span>
    </div>
  );
}

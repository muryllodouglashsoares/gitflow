import { useState, type ReactNode } from "react";
import type { FlowLogEntry } from "../../lib/importFlow";
import { stepperFor, type ImportStage } from "../../lib/importFlow";
import { Button } from "../common/Button";
import { Spinner } from "../common/Spinner";

/** Seção com título, no mesmo estilo dos cartões das outras telas. */
export function Card({
  title,
  children,
  tone = "default",
  className = "",
}: {
  title?: string;
  children: ReactNode;
  tone?: "default" | "warning" | "danger" | "success";
  className?: string;
}) {
  const border = {
    default: "border-line bg-surface",
    warning: "border-modified/40 bg-modified/10",
    danger: "border-removed/40 bg-removed/10",
    success: "border-added/40 bg-added/10",
  }[tone];
  return (
    <section className={`rounded-md border p-4 text-sm ${border} ${className}`}>
      {title && <h3 className="mb-3 font-semibold">{title}</h3>}
      {children}
    </section>
  );
}

/** Caixa de seleção com rótulo clicável. */
export function Check({
  checked,
  onChange,
  disabled,
  children,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <label className={`flex items-start gap-2.5 ${disabled ? "opacity-60" : "cursor-pointer"}`}>
      <input
        type="checkbox"
        className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--color-accent)]"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span>{children}</span>
    </label>
  );
}

export function Chip({ label, value, tone }: { label: string; value: ReactNode; tone?: string }) {
  return (
    <div className="rounded-md border border-line bg-canvas/40 px-3 py-1.5">
      <div className={`text-base font-semibold ${tone ?? ""}`}>{value}</div>
      <div className="text-xs text-muted">{label}</div>
    </div>
  );
}

/** Progresso INDETERMINADO: o backend não informa porcentagem real, então nenhuma é inventada. */
export function Working({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <Card>
      <div role="status" aria-live="polite" className="flex items-start gap-3">
        <Spinner className="mt-0.5 shrink-0 text-accent" />
        <div>
          <p className="font-medium">{title}</p>
          {children && <div className="mt-1 text-muted">{children}</div>}
        </div>
      </div>
    </Card>
  );
}

/** Botão que pede uma confirmação inline explicando o efeito antes de executar. */
export function ConfirmButton({
  label,
  confirmLabel,
  explanation,
  disabled,
  onConfirm,
}: {
  label: string;
  confirmLabel: string;
  explanation: ReactNode;
  disabled?: boolean;
  onConfirm: () => void;
}) {
  const [open, setOpen] = useState(false);
  if (!open) {
    return (
      <Button variant="secondary" disabled={disabled} onClick={() => setOpen(true)}>
        {label}
      </Button>
    );
  }
  return (
    <div role="group" aria-label={label} className="rounded-md border border-modified/40 bg-modified/10 p-3 text-sm">
      <div className="text-fg">{explanation}</div>
      <div className="mt-3 flex gap-2">
        <Button
          variant="primary"
          disabled={disabled}
          onClick={() => {
            setOpen(false);
            onConfirm();
          }}
        >
          {confirmLabel}
        </Button>
        <Button variant="secondary" onClick={() => setOpen(false)}>
          Cancelar
        </Button>
      </div>
    </div>
  );
}

export function PathList({ paths, max = 12 }: { paths: readonly string[]; max?: number }) {
  if (paths.length === 0) return null;
  return (
    <ul className="mt-1.5 max-h-40 overflow-auto font-mono text-xs break-all text-muted">
      {paths.slice(0, max).map((path) => (
        <li key={path}>{path}</li>
      ))}
      {paths.length > max && <li className="text-faint">… e mais {paths.length - max}</li>}
    </ul>
  );
}

export function Stepper({ stage }: { stage: ImportStage }) {
  const steps = stepperFor(stage);
  return (
    <ol className="flex flex-wrap items-center gap-x-1 gap-y-1 text-xs" aria-label="Etapas da importação">
      {steps.map((step, index) => (
        <li
          key={step.id}
          aria-current={step.state === "current" ? "step" : undefined}
          className="flex items-center gap-1"
        >
          <span
            className={`inline-flex h-5 w-5 items-center justify-center rounded-full text-[11px] font-semibold ${
              step.state === "done"
                ? "bg-added/20 text-added"
                : step.state === "current"
                  ? "bg-accent text-accent-fg"
                  : "bg-raised text-faint"
            }`}
          >
            {step.state === "done" ? "✓" : index + 1}
          </span>
          <span className={step.state === "current" ? "font-medium text-fg" : "text-muted"}>{step.label}</span>
          {index < steps.length - 1 && <span className="mx-1 text-faint" aria-hidden="true">›</span>}
        </li>
      ))}
    </ol>
  );
}

const OUTCOME_LABEL: Record<FlowLogEntry["outcome"], string> = {
  started: "iniciada",
  ok: "concluída",
  failed: "falhou",
  info: "info",
};
const OUTCOME_TONE: Record<FlowLogEntry["outcome"], string> = {
  started: "text-muted",
  ok: "text-added",
  failed: "text-removed",
  info: "text-faint",
};

/** Registro auditável das etapas: o que foi feito, com que resultado e em que horário. */
export function OperationLog({ log }: { log: readonly FlowLogEntry[] }) {
  if (log.length === 0) return null;
  return (
    <details className="rounded-md border border-line bg-surface p-3 text-sm">
      <summary className="cursor-pointer select-none text-muted hover:text-fg">
        Registro da operação ({log.length})
      </summary>
      <ol className="mt-2 max-h-56 space-y-1 overflow-auto font-mono text-xs">
        {log.map((entry, index) => (
          <li key={index} className="flex flex-wrap gap-x-2">
            <span className="text-faint">{new Date(entry.at).toLocaleTimeString("pt-BR")}</span>
            <span>{entry.step}</span>
            <span className={OUTCOME_TONE[entry.outcome]}>{OUTCOME_LABEL[entry.outcome]}</span>
            {entry.detail && <span className="break-all text-muted">{entry.detail}</span>}
          </li>
        ))}
      </ol>
    </details>
  );
}

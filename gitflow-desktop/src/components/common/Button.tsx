import type { ButtonHTMLAttributes } from "react";

type Variant = "primary" | "secondary";

const VARIANTS: Record<Variant, string> = {
  primary: "bg-accent text-accent-fg hover:brightness-110 disabled:opacity-60 disabled:hover:brightness-100",
  secondary:
    "border border-line bg-raised text-fg hover:border-faint disabled:opacity-60 disabled:hover:border-line",
};

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
}

export function Button({ variant = "secondary", className = "", type = "button", ...props }: ButtonProps) {
  return (
    <button
      type={type}
      className={`inline-flex items-center justify-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition-colors ${VARIANTS[variant]} ${className}`}
      {...props}
    />
  );
}

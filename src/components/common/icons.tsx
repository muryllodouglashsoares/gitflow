import type { SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement>;

function Icon({ children, ...props }: IconProps) {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...props}
    >
      {children}
    </svg>
  );
}

export const FolderOpenIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M1.75 4.25a1 1 0 0 1 1-1h3.1l1.4 1.5h5a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-9.5a1 1 0 0 1-1-1z" />
  </Icon>
);

export const RefreshIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M13.25 8a5.25 5.25 0 1 1-1.54-3.71" />
    <path d="M13.25 2.5v2.75H10.5" />
  </Icon>
);

export const BranchIcon = (p: IconProps) => (
  <Icon {...p}>
    <circle cx="4.5" cy="3.5" r="1.5" />
    <circle cx="4.5" cy="12.5" r="1.5" />
    <circle cx="11.5" cy="5.5" r="1.5" />
    <path d="M4.5 5v6" />
    <path d="M11.5 7c0 2.5-3.5 2-6 4" />
  </Icon>
);

export const ChangesIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect x="2.25" y="2.25" width="11.5" height="11.5" rx="1.5" />
    <path d="M5 8h6M8 5v6" />
  </Icon>
);

export const CheckCircleIcon = (p: IconProps) => (
  <Icon {...p}>
    <circle cx="8" cy="8" r="6" />
    <path d="m5.5 8.2 1.8 1.8 3.2-3.5" />
  </Icon>
);

export const AlertIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M8 2.2 14.2 13H1.8z" />
    <path d="M8 6.5v3" />
    <path d="M8 11.4v.1" />
  </Icon>
);

export const FileIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M4 1.75h5l3 3v9.5H4z" />
    <path d="M9 1.75v3h3" />
  </Icon>
);

export const CloseIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="m4 4 8 8M12 4l-8 8" />
  </Icon>
);

/** Logotipo simples (ramificação) usado na barra lateral e na tela inicial. */
export const LogoMark = (p: IconProps) => (
  <svg width="22" height="22" viewBox="0 0 24 24" fill="none" aria-hidden="true" focusable="false" {...p}>
    <rect width="24" height="24" rx="6" fill="var(--color-raised)" />
    <g stroke="var(--color-accent)" strokeWidth="1.8" strokeLinecap="round">
      <path d="M8.5 6.5v11" />
      <path d="M8.5 13.5 15.5 9" />
    </g>
    <g fill="var(--color-raised)" stroke="var(--color-accent)" strokeWidth="1.8">
      <circle cx="8.5" cy="6" r="1.8" />
      <circle cx="8.5" cy="18" r="1.8" />
      <circle cx="15.5" cy="8.5" r="1.8" />
    </g>
  </svg>
);

export const SyncIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M2.5 6.5a5.5 5.5 0 0 1 9.4-2.2L13.5 6" />
    <path d="M13.5 2.8V6h-3.2" />
    <path d="M13.5 9.5a5.5 5.5 0 0 1-9.4 2.2L2.5 10" />
    <path d="M2.5 13.2V10h3.2" />
  </Icon>
);

/** Pacote/arquivo compactado (importação de ZIP). */
export const ImportIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M2.5 3.25a1 1 0 0 1 1-1h9a1 1 0 0 1 1 1v9.5a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1z" />
    <path d="M8 2.25v3m0 1.5v1.5m0 1.5v1.5" />
    <path d="M6.25 11.25h3.5" />
  </Icon>
);

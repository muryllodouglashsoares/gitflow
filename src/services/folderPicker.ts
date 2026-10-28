import { open } from "@tauri-apps/plugin-dialog";
import { isTauri } from "@tauri-apps/api/core";

/**
 * Abre o seletor de pastas nativo do sistema.
 * Retorna `null` quando o usuário cancela.
 */
export async function pickFolder(): Promise<string | null> {
  if (!isTauri()) {
    throw new Error(
      "O seletor de pastas só funciona no aplicativo desktop. Execute \"npm run tauri:dev\".",
    );
  }
  const selected = await open({
    directory: true,
    multiple: false,
    title: "Selecione a pasta do repositório",
  });
  return typeof selected === "string" ? selected : null;
}

/**
 * Abre o seletor de arquivos nativo restrito a `.zip` (Fase 4).
 * O caminho devolvido é tratado como entrada NÃO confiável: o backend o valida de novo.
 * Retorna `null` quando o usuário cancela.
 */
export async function pickZipFile(): Promise<string | null> {
  if (!isTauri()) {
    throw new Error(
      "O seletor de arquivos só funciona no aplicativo desktop. Execute \"npm run tauri:dev\".",
    );
  }
  const selected = await open({
    directory: false,
    multiple: false,
    title: "Selecione o arquivo ZIP da atualização",
    filters: [{ name: "Arquivo ZIP", extensions: ["zip"] }],
  });
  return typeof selected === "string" ? selected : null;
}

import { describe, expect, it } from "vitest";
import { isGitInstallationError, isRemoteUnverifiable, toGitError } from "./errors";

describe("toGitError", () => {
  it("preserva o erro serializado pelo Rust", () => {
    expect(
      toGitError({ code: "NOT_A_REPOSITORY", message: "A pasta selecionada não é um repositório Git." }),
    ).toEqual({ code: "NOT_A_REPOSITORY", message: "A pasta selecionada não é um repositório Git." });
    expect(toGitError({ code: "COMMAND_FAILED", message: "falhou", details: "fatal: x" }).details).toBe(
      "fatal: x",
    );
  });

  it("aceita strings, Error e valores inesperados sem lançar", () => {
    expect(toGitError("boom")).toEqual({ code: "FRONTEND", message: "boom" });
    expect(toGitError(new Error("falha"))).toEqual({ code: "FRONTEND", message: "falha" });
    expect(toGitError(undefined).code).toBe("FRONTEND");
    expect(toGitError({ code: "ESTRANHO", message: "x" }).code).toBe("FRONTEND");
    expect(toGitError({}).message.length).toBeGreaterThan(0);
  });

  it("identifica erros de instalação do Git", () => {
    expect(isGitInstallationError({ code: "GIT_NOT_FOUND", message: "" })).toBe(true);
    expect(isGitInstallationError({ code: "NOT_A_REPOSITORY", message: "" })).toBe(false);
  });

  it("reconhece os códigos de erro da Fase 3 e os que tornam o remoto não verificável", () => {
    for (const code of ["AUTH_FAILED", "NETWORK_ERROR", "PUSH_REJECTED", "MERGE_CONFLICT"]) {
      expect(toGitError({ code, message: "x" }).code).toBe(code);
    }
    expect(isRemoteUnverifiable({ code: "NETWORK_ERROR", message: "" })).toBe(true);
    expect(isRemoteUnverifiable({ code: "TIMEOUT", message: "" })).toBe(true);
    expect(isRemoteUnverifiable({ code: "PUSH_REJECTED", message: "" })).toBe(false);
    expect(isRemoteUnverifiable({ code: "MERGE_CONFLICT", message: "" })).toBe(false);
  });
});

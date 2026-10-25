@echo off
setlocal EnableExtensions
title GitFlow Desktop
cd /d "%~dp0"

rem ==========================================================================
rem  GitFlow Desktop - inicializador para Windows 10/11
rem  Duplo clique: verifica/instala Git, Node.js, Visual Studio Build Tools
rem  (C++), WebView2 e Rust, instala as dependencias do projeto e abre o app.
rem  Mantenha este arquivo na raiz do projeto (ao lado de package.json).
rem ==========================================================================

echo ============================================================
echo   GitFlow Desktop - inicializador
echo ============================================================
echo.

if not exist "package.json" (
  echo [ERRO] package.json nao encontrado. Deixe este arquivo na raiz do projeto.
  goto :fail
)

call :refresh_path
echo Verificando requisitos...
call :check_all

if "%OK_GIT%"=="0"  echo   - Git 2.25 ou mais novo: AUSENTE
if "%OK_NODE%"=="0" echo   - Node.js 20 ou mais novo: AUSENTE
if "%OK_VS%"=="0"   echo   - Visual Studio Build Tools com C++: AUSENTE
if "%OK_WV%"=="0"   echo   - WebView2 Runtime: AUSENTE
if "%OK_RUST%"=="0" echo   - Rust 1.80 ou mais novo, toolchain MSVC: AUSENTE

set "NEED_ADMIN=0"
if "%OK_GIT%"=="0"  set "NEED_ADMIN=1"
if "%OK_NODE%"=="0" set "NEED_ADMIN=1"
if "%OK_VS%"=="0"   set "NEED_ADMIN=1"
if "%OK_WV%"=="0"   set "NEED_ADMIN=1"

if "%NEED_ADMIN%"=="0" goto :rust_step

net session >nul 2>&1
if errorlevel 1 goto :elevate
goto :install_system

rem --------------------------------------------------------------------------
rem  Pede permissao de administrador (so quando falta algo do sistema)
rem --------------------------------------------------------------------------
:elevate
if /i "%~1"=="elevado" (
  echo [ERRO] Nao foi possivel obter privilegios de administrador.
  goto :fail
)
echo.
echo Faltam componentes que exigem permissao de administrador. Solicitando...
powershell -NoProfile -Command "Start-Process -FilePath '%~f0' -ArgumentList 'elevado' -Verb RunAs" >nul 2>&1
if errorlevel 1 (
  echo [ERRO] Permissao negada. Execute de novo e aceite o aviso do Windows.
  goto :fail
)
exit /b 0

rem --------------------------------------------------------------------------
rem  Instala componentes do sistema via winget (Git, Node, WebView2, VS Tools)
rem --------------------------------------------------------------------------
:install_system
echo.
echo Instalando componentes do sistema. Isso pode levar varios minutos...
where winget >nul 2>&1
if errorlevel 1 (
  echo [ERRO] O winget nao foi encontrado. Instale o App Installer pela Microsoft Store
  echo        ou instale manualmente: Git, Node.js 20+, Visual Studio Build Tools 2022
  echo        com C++ e WebView2 Runtime. Depois execute este arquivo de novo.
  goto :fail
)

if "%OK_GIT%"=="0" (
  echo.
  echo [Git]
  call :winget_ensure Git.Git
)
if "%OK_NODE%"=="0" (
  echo.
  echo [Node.js]
  call :winget_ensure OpenJS.NodeJS.LTS
)
if "%OK_WV%"=="0" (
  echo.
  echo [WebView2]
  call :winget_ensure Microsoft.EdgeWebView2Runtime
)
if "%OK_VS%"=="0" (
  echo.
  echo [Visual Studio Build Tools] Download grande, mantenha a janela aberta...
  winget install -e --id Microsoft.VisualStudio.2022.BuildTools --accept-package-agreements --accept-source-agreements --override "--passive --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
)

call :refresh_path
call :check_all
if "%OK_VS%"=="0" call :fix_vs
call :refresh_path
call :check_all

set "FAILED="
if "%OK_GIT%"=="0"  set "FAILED=%FAILED% Git"
if "%OK_NODE%"=="0" set "FAILED=%FAILED% Node.js"
if "%OK_VS%"=="0"   set "FAILED=%FAILED% VS-Build-Tools"
if "%OK_WV%"=="0"   set "FAILED=%FAILED% WebView2"
if defined FAILED (
  echo.
  echo [ERRO] Nao foi possivel concluir:%FAILED%
  echo        Se o instalador pediu reinicializacao, reinicie o Windows e abra este arquivo de novo.
  goto :fail
)

echo.
echo Componentes do sistema instalados. Reabrindo sem privilegios de administrador...
start "" explorer.exe "%~f0"
exit /b 0

rem --------------------------------------------------------------------------
rem  Rust (instalacao por usuario, nao exige administrador)
rem --------------------------------------------------------------------------
:rust_step
if "%OK_RUST%"=="1" goto :deps_step
echo.
echo [Rust] Instalando...
call :install_rust
call :refresh_path
call :check_rust
if "%OK_RUST%"=="0" (
  echo [ERRO] O Rust nao foi instalado corretamente.
  goto :fail
)

rem --------------------------------------------------------------------------
rem  Dependencias do projeto (so reinstala se package*.json mudou)
rem --------------------------------------------------------------------------
:deps_step
if not exist "node_modules\.gitflow-instalado" goto :do_npm
powershell -NoProfile -Command "$s=(Get-Item 'node_modules\.gitflow-instalado').LastWriteTime; if (((Get-Item 'package.json').LastWriteTime -gt $s) -or ((Get-Item 'package-lock.json').LastWriteTime -gt $s)) { exit 1 }"
if errorlevel 1 goto :do_npm
goto :run_app

:do_npm
echo.
echo Instalando dependencias do projeto (npm install)...
call npm install --no-audit --no-fund
if errorlevel 1 (
  echo [ERRO] Falha no npm install. Verifique a conexao com a internet.
  goto :fail
)
echo ok>"node_modules\.gitflow-instalado"

rem --------------------------------------------------------------------------
rem  Executa o aplicativo (esta janela fica com o sistema rodando)
rem --------------------------------------------------------------------------
:run_app
netstat -ano | findstr /R /C:":1420 .*LISTENING" >nul
if not errorlevel 1 (
  echo [ERRO] A porta 1420 ja esta em uso. Feche outra instancia do aplicativo e tente de novo.
  goto :fail
)
echo.
echo Iniciando o GitFlow Desktop.
echo Na primeira execucao a compilacao do Rust pode levar varios minutos.
echo Mantenha esta janela aberta enquanto usa o aplicativo.
echo.
call npm run tauri:dev
set "RC=%errorlevel%"
echo.
echo Aplicativo encerrado. Codigo de saida: %RC%
pause
exit /b %RC%

:fail
echo.
pause
exit /b 1

rem ==========================================================================
rem  Sub-rotinas
rem ==========================================================================

:refresh_path
rem Recarrega o PATH do registro (necessario logo apos instalar algo)
for /f "usebackq delims=" %%P in (`powershell -NoProfile -Command "[Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User')"`) do set "PATH=%%P"
if exist "%USERPROFILE%\.cargo\bin" set "PATH=%PATH%;%USERPROFILE%\.cargo\bin"
if exist "%ProgramFiles%\Git\cmd\git.exe" set "PATH=%PATH%;%ProgramFiles%\Git\cmd"
exit /b 0

:check_all
call :chk_git
call :chk_node
call :chk_vs
call :chk_wv
call :check_rust
exit /b 0

:chk_git
set "OK_GIT=0"
set "GV=" & set "GMAJ=" & set "GMIN="
where git >nul 2>&1
if errorlevel 1 exit /b 0
for /f "tokens=3" %%v in ('git --version 2^>nul') do if not defined GV set "GV=%%v"
for /f "tokens=1,2 delims=." %%a in ("%GV%") do (set "GMAJ=%%a" & set "GMIN=%%b")
if not defined GMIN exit /b 0
if %GMAJ% GTR 2 set "OK_GIT=1"
if %GMAJ% EQU 2 if %GMIN% GEQ 25 set "OK_GIT=1"
exit /b 0

:chk_node
set "OK_NODE=0"
set "NV="
where node >nul 2>&1
if errorlevel 1 exit /b 0
where npm >nul 2>&1
if errorlevel 1 exit /b 0
for /f "tokens=1 delims=." %%a in ('node -v 2^>nul') do if not defined NV set "NV=%%a"
if not defined NV exit /b 0
set "NV=%NV:v=%"
if %NV% GEQ 20 set "OK_NODE=1"
exit /b 0

:chk_vs
rem Precisa do componente MSVC (C++) e do Windows SDK
set "OK_VS=0"
set "VSP="
set "VSW=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSW%" exit /b 0
for /f "usebackq delims=" %%i in (`"%VSW%" -products * -latest -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSP=%%i"
if not defined VSP exit /b 0
if not exist "%ProgramFiles(x86)%\Windows Kits\10\Lib" exit /b 0
set "OK_VS=1"
exit /b 0

:chk_wv
set "OK_WV=0"
reg query "HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" /v pv >nul 2>&1
if not errorlevel 1 set "OK_WV=1"
reg query "HKLM\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" /v pv >nul 2>&1
if not errorlevel 1 set "OK_WV=1"
reg query "HKCU\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" /v pv >nul 2>&1
if not errorlevel 1 set "OK_WV=1"
exit /b 0

:check_rust
set "OK_RUST=0"
set "RV=" & set "RMAJ=" & set "RMIN="
where cargo >nul 2>&1
if errorlevel 1 exit /b 0
where rustc >nul 2>&1
if errorlevel 1 exit /b 0
rustc -vV 2>nul | findstr /C:"windows-msvc" >nul
if errorlevel 1 exit /b 0
for /f "tokens=2" %%v in ('rustc --version 2^>nul') do if not defined RV set "RV=%%v"
for /f "tokens=1,2 delims=." %%a in ("%RV%") do (set "RMAJ=%%a" & set "RMIN=%%b")
if not defined RMIN exit /b 0
if %RMAJ% GTR 1 set "OK_RUST=1"
if %RMAJ% EQU 1 if %RMIN% GEQ 80 set "OK_RUST=1"
exit /b 0

:winget_ensure
rem Instala; se ja existir (versao antiga), tenta atualizar
winget install -e --id %~1 --silent --accept-package-agreements --accept-source-agreements
if not "%errorlevel%"=="0" winget upgrade -e --id %~1 --silent --accept-package-agreements --accept-source-agreements
exit /b 0

:fix_vs
rem Build Tools ja existia sem a carga C++: adiciona a carga ao que esta instalado
set "VSANY="
if not exist "%VSW%" exit /b 0
for /f "usebackq delims=" %%i in (`"%VSW%" -products * -latest -property installationPath`) do set "VSANY=%%i"
if not defined VSANY exit /b 0
echo Adicionando a carga C++ ao Visual Studio ja instalado...
"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\setup.exe" modify --installPath "%VSANY%" --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive --norestart --wait
exit /b 0

:install_rust
where rustup >nul 2>&1
if not errorlevel 1 goto :rust_update
echo Baixando o instalador do Rust...
powershell -NoProfile -Command "try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; Invoke-WebRequest -UseBasicParsing -Uri 'https://win.rustup.rs/x86_64' -OutFile (Join-Path $env:TEMP 'rustup-init.exe') } catch { exit 1 }"
if errorlevel 1 exit /b 1
"%TEMP%\rustup-init.exe" -y --profile minimal --default-toolchain stable-x86_64-pc-windows-msvc
exit /b 0
:rust_update
rustup toolchain install stable-x86_64-pc-windows-msvc --profile minimal
rustup default stable-x86_64-pc-windows-msvc
exit /b 0

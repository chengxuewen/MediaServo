@echo off
REM mediaservo.bat — MediaServo CLI 薄壳（Windows，best-effort）
REM 职责: ① 检测 pixi（缺失提示 bootstrap）② 激活环境 ③ 转发到 CLI
REM 注意: pixi.toml platforms 无 win-64 — 激活可能失败（v2 审核 HIGH），提示明确
set "ROOT=%~dp0"

if not exist "%USERPROFILE%\.pixi\bin\pixi.exe" (
    echo pixi 未安装 — 先运行: bootstrap.bat
    exit /b 1
)

call "%ROOT%scripts\pixi-shell.bat"
if errorlevel 1 (
    echo 激活失败: pixi.toml platforms 需含 win-64（当前不支持 Windows，见 docs/plans/build-cli/plan.md）
    exit /b 1
)

python "%ROOT%scripts\mediaservo_cli.py" %*
exit /b %errorlevel%

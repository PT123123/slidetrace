# slidetrace — 构建与运行
#
# 约定（重要）：
#   just build  → 只负责构建
#   just run    → 只负责运行「已经构建好的产物」，绝不触发构建
#
# 用法示例：
#   just build
#   just run
#   just run --selftest
#   just run -- --root target/verify-root --selftest
#
# 目标平台：Windows / x86_64-pc-windows-msvc

set shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

bin := "slidetrace"
debug_bin   := "target/debug/"   + bin + ".exe"
release_bin := "target/release/" + bin + ".exe"

# 列出所有可用命令
default:
    @just --list

# 构建 debug 产物（等价于 cargo build）
build:
    cargo build

# 构建 release 产物（等价于 cargo build --release）
build-release:
    cargo build --release

# 运行已构建的 debug 产物。不会构建；产物不存在时直接报错。
run *args:
    $bin = '{{debug_bin}}'; if (-not (Test-Path $bin)) { Write-Host "[just run] 未找到 $bin —— 请先执行: just build" -ForegroundColor Red; exit 1 }; $rest = @(); $raw = '{{args}}'.Trim(); if ($raw.Length -gt 0) { $rest = $raw -split '\s+'; if ($rest[0] -eq '--') { $rest = @($rest | Select-Object -Skip 1) } }; & $bin @rest; exit $LASTEXITCODE

# 运行已构建的 release 产物。不会构建；产物不存在时直接报错。
run-release *args:
    $bin = '{{release_bin}}'; if (-not (Test-Path $bin)) { Write-Host "[just run-release] 未找到 $bin —— 请先执行: just build-release" -ForegroundColor Red; exit 1 }; $rest = @(); $raw = '{{args}}'.Trim(); if ($raw.Length -gt 0) { $rest = $raw -split '\s+'; if ($rest[0] -eq '--') { $rest = @($rest | Select-Object -Skip 1) } }; & $bin @rest; exit $LASTEXITCODE

# 运行单元测试（会先编译测试目标）
test:
    cargo test

# 快速类型检查，不产出可执行文件
check:
    cargo check

# 清理构建产物
clean:
    cargo clean
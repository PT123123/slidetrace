//! slidetrace —— 以讲话时间为主轴的演讲创作与演练工作台。
//!
//! 入口职责：解析命令行、准备项目目录、装配 Slint 组件。
//! 业务逻辑在 `app` / `timeline` / `model` / `audio` / `storage` 里。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod camera;
mod demo;
mod model;
mod selftest;
mod storage;
mod timeline;

use std::path::PathBuf;
use std::time::Duration;

use app::Mode;
use storage::{ProjectHandle, Store};

const HELP: &str = "\
slidetrace —— 以讲话时间为主轴的演讲创作与演练工作台

用法：
  slidetrace [选项]

选项：
  --root <目录>             覆盖项目根目录（默认 %APPDATA%\\slidetrace）
  --project <slug>          启动时打开指定项目
  --reset                   清空项目根目录后重建演示项目
  --screenshot <文件.png>   开发用：渲染完成后截图存 PNG 并退出
  --screenshot-mode <模式>  截图模式：create|rehearse|review|present（默认 create）
  --screenshot-delay <毫秒> 截图前等待时间（默认 1500）
  --screenshot-crop x,y,w,h 截图时只导出该矩形区域（便于核对某个面板）
  --selftest                开发用：用真实状态机跑一遍核心闭环并逐条断言
  -h, --help                显示本帮助

快捷键：
  制作模式  文字/图片/选择工具 · 双击文字改内容 · 拖动移动 · 拖右下角改尺寸
  演练模式  Ctrl+R 开始/停止录制 · 1-9 触发元素出现 · 空格触发下一个元素
            Esc 停止录制 · M 打标记 · H 隐藏最近出现的元素 · 鼠标拖动 = 手写
  回放模式  空格 播放/暂停 · 拖动时间轴定位 · 点击元素或笔迹跳回当时讲话时间
            点击讲稿某一句即可跳转到该句
  演讲模式  ← / → 翻页 · 1-9 / 空格 触发元素 · Esc 退出演讲
  任意模式  Ctrl+N 新建项目 · Ctrl+O 打开项目 · Ctrl+S 保存 · Ctrl+I 检查器
";

#[derive(Debug)]
struct Args {
    root: Option<PathBuf>,
    project: Option<String>,
    reset: bool,
    screenshot: Option<PathBuf>,
    screenshot_mode: Mode,
    screenshot_delay: Duration,
    screenshot_crop: Option<(u32, u32, u32, u32)>,
    selftest: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            root: None,
            project: None,
            reset: false,
            screenshot: None,
            screenshot_mode: Mode::Create,
            screenshot_delay: Duration::from_millis(1500),
            screenshot_crop: None,
            selftest: false,
        }
    }
}

fn parse_args(argv: &[String]) -> Result<Option<Args>, String> {
    let mut a = Args::default();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--root" => {
                i += 1;
                a.root = Some(PathBuf::from(
                    argv.get(i).ok_or("--root 缺少参数值")?.clone(),
                ));
            }
            "--project" => {
                i += 1;
                a.project = Some(argv.get(i).ok_or("--project 缺少参数值")?.clone());
            }
            "--reset" => a.reset = true,
            "--screenshot" => {
                i += 1;
                a.screenshot = Some(PathBuf::from(
                    argv.get(i).ok_or("--screenshot 缺少参数值")?.clone(),
                ));
            }
            "--screenshot-mode" => {
                i += 1;
                let v = argv.get(i).ok_or("--screenshot-mode 缺少参数值")?.clone();
                a.screenshot_mode = match v.as_str() {
                    "create" => Mode::Create,
                    "rehearse" => Mode::Rehearse,
                    "review" => Mode::Review,
                    "present" => Mode::Present,
                    other => return Err(format!("未知模式：{other}")),
                };
            }
            "--screenshot-delay" => {
                i += 1;
                let v = argv.get(i).ok_or("--screenshot-delay 缺少参数值")?.clone();
                let ms: u64 = v.parse().map_err(|_| format!("不是合法毫秒数：{v}"))?;
                a.screenshot_delay = Duration::from_millis(ms);
            }
            "--screenshot-crop" => {
                i += 1;
                let v = argv.get(i).ok_or("--screenshot-crop 缺少参数值")?.clone();
                let parts: Vec<u32> = v
                    .split(',')
                    .map(|s| s.trim().parse::<u32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| format!("--screenshot-crop 需要 x,y,w,h 四个整数：{v}"))?;
                if parts.len() != 4 {
                    return Err(format!("--screenshot-crop 需要 x,y,w,h 四个整数：{v}"));
                }
                a.screenshot_crop = Some((parts[0], parts[1], parts[2], parts[3]));
            }
            "--selftest" => a.selftest = true,
            "-h" | "--help" => return Ok(None),
            other => return Err(format!("未知参数：{other}")),
        }
        i += 1;
    }
    Ok(Some(a))
}

/// 准备一个可用项目：优先打开指定/已有项目，否则装一个演示项目。
fn prepare_project(store: &Store, args: &Args) -> ProjectHandle {
    if args.reset {
        let dir = store.projects_dir();
        if dir.exists() {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => println!("[init] 已清空 {}", dir.display()),
                Err(e) => eprintln!("[warn] 清空 {} 失败：{e}", dir.display()),
            }
        }
    }
    let _ = store.ensure_root();

    if let Some(slug) = &args.project {
        match store.open_project(slug) {
            Ok(h) => return h,
            Err(e) => eprintln!("[warn] 打开项目 {slug} 失败：{e}，改为自动选择"),
        }
    }

    let existing = store.list_projects();
    if let Some((slug, _)) = existing.first() {
        if let Ok(h) = store.open_project(slug) {
            println!("[init] 打开已有项目「{}」", h.project().name);
            return h;
        }
    }

    // 首次启动：生成演示项目（3 页 / 8 个元素 / 5 段讲稿 / 一次合成音频的演练）
    match demo::install(store) {
        Ok(h) => {
            println!(
                "[init] 已创建演示项目「{}」于 {}",
                h.project().name,
                h.dir().display()
            );
            h
        }
        Err(e) => {
            eprintln!("[warn] 创建演示项目失败：{e}，退化为空白项目");
            store
                .create_project("未命名项目")
                .expect("无法创建项目目录")
        }
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(Some(a)) => a,
        Ok(None) => {
            print!("{HELP}");
            return;
        }
        Err(e) => {
            eprintln!("参数错误：{e}\n\n{HELP}");
            std::process::exit(2);
        }
    };

    let root = args.root.clone().unwrap_or_else(Store::default_root);
    let store = Store::new(root.clone());
    println!("[init] 项目根目录：{}", root.display());

    let handle = prepare_project(&store, &args);
    println!(
        "[init] 项目「{}」：{} 页 / {} 段讲稿 / {} 次演练",
        handle.project().name,
        handle.project().pages.len(),
        handle.project().script.sections.len(),
        handle.project().rehearsals.len()
    );
    match audio::record::input_device_available() {
        Some(name) => println!("[init] 麦克风：{name}"),
        None => println!("[init] 未检测到麦克风 —— 仍可演练，只是没有音频"),
    }

    if args.selftest {
        match selftest::run(store, handle) {
            Ok(()) => return,
            Err(e) => {
                eprintln!("自检失败：{e}");
                std::process::exit(1);
            }
        }
    }

    let result = match args.screenshot {
        Some(out) => {
            println!(
                "[截图] 模式 {:?}，等待 {:?} 后写入 {}",
                args.screenshot_mode,
                args.screenshot_delay,
                out.display()
            );
            app::run_screenshot(
                store,
                handle,
                args.screenshot_mode,
                out,
                args.screenshot_delay,
                args.screenshot_crop,
            )
            .map_err(|e| e.to_string())
        }
        None => {
            println!("\n{HELP}");
            app::run_app(store, handle).map_err(|e| e.to_string())
        }
    };

    if let Err(e) = result {
        eprintln!("运行失败：{e}");
        std::process::exit(1);
    }
}

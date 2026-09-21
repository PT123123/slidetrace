# slidetrace

> **以「讲话时间」为主轴**的演讲创作与演练工作台。
> 不是「带录音的 PPT」，而是把讲稿、视觉元素、手写、音频绑定到**同一条时间轴**上，
> 形成双向联动：我说到哪里 → 内容出现到哪里；我点击某个东西 → 听到当时说的话。

Rust 2021 + Slint 1.18 的 Windows 桌面应用。产品规格见 [`docs/SPEC-v1.md`](docs/SPEC-v1.md)。

---

## 1. 构建与运行

环境：`rustc 1.98` / `cargo 1.98`，`x86_64-pc-windows-msvc`，**纯 Rust 可构建**
（无 cmake / C++ / 外部二进制依赖）。

```bash
cargo build                     # 编译（默认特性）
cargo test                      # 66 个单元测试
cargo run                       # 启动窗口（首次会自动创建演示项目）
cargo run -- --reset            # 清空项目目录并重建演示项目
cargo run -- --help             # 命令行参数 + 快捷键帮助
```

### 用 `just` 构建与运行

仓库自带 [`justfile`](justfile)（[just](https://github.com/casey/just) 1.58 实测通过）：

```bash
just build            # 只构建：等价于 cargo build
just run              # 只运行「已构建」的产物，不触发构建
just run --selftest   # 参数透传给应用（写成 just run -- --selftest 也可以）
just run-release      # 运行 release 产物（先执行 just build-release）
just test / just check / just clean
```

> `just run` **不做任何构建**：它直接执行 `target/debug/slidetrace.exe`。
> 产物不存在时会立即报错并提示先执行 `just build`（退出码 1），而不是悄悄替你构建。

### 开发/自动化用的额外参数

| 参数 | 作用 |
| --- | --- |
| `--root <目录>` | 覆盖项目根目录（默认 `%APPDATA%\slidetrace`），便于隔离测试 |
| `--project <slug>` | 启动时打开指定项目 |
| `--reset` | 清空项目根目录后重建演示项目 |
| `--screenshot <文件.png>` | 渲染完成后用 `Window::take_snapshot()` 存 PNG 并退出 |
| `--screenshot-mode <模式>` | 截图时的模式：`create\|rehearse\|review\|present` |
| `--screenshot-delay <毫秒>` | 截图前等待时间（默认 1500，等待首帧与定时器刷新） |
| `--screenshot-crop x,y,w,h` | 只导出该矩形区域，方便核对某个面板 |
| `--selftest` | **端到端自检**：用真实状态机跑一遍核心闭环并逐条断言（45 项） |

> 说明：`release` 构建带 `windows_subsystem = "windows"`，不会弹控制台，
> 因此上面这些参数的输出只在 `debug`（`cargo run`）构建里可见。

---

## 2. 首次启动会发生什么

如果项目根目录下没有任何项目，程序会自动创建一个**演示项目**：

```
示例演讲：以讲话时间为主轴
├── 3 页，每页 2-3 个文字/图片元素（图片是程序生成的占位 PNG）
├── 5 段讲稿，每段都有 Full / Prompt / Minimal 三档文本
└── 1 次演练「示例演练（合成音频）」
    ├── audio.wav       36.5s 的合成语音包络（16kHz 单声道）
    ├── rehearsal.json  14 个时间轴事件 + 4 条带逐点时间的笔迹
    └── 讲稿锚点        s1..s5 各自的讲话时间
```

**为什么要带一次合成音频的演练**：这样在没有麦克风的机器（CI、无声卡环境）上，
第一次打开就能立刻拖动时间轴、看到笔迹按时间"长出来"、点击元素跳回讲话位置——
完整闭环可验证，而不是只能看到一个空壳。

---

## 3. 快捷键

### 通用

| 快捷键 | 作用 |
| --- | --- |
| `Ctrl+R` | 开始 / 停止录制（等同于点 Record 按钮） |
| `Ctrl+P` | 播放 / 暂停（回放模式） |
| `Ctrl+N` / `Ctrl+O` / `Ctrl+S` | 新建项目 / 打开项目 / 保存 |
| `Ctrl+I` | 展开或收起右侧 Inspector |
| `Esc` | 录制中 → 停止录制；否则回到「制作」模式 |

### 制作（Create）

| 操作 | 作用 |
| --- | --- |
| 工具栏「选择 / 文字 / 图片」 | 切换工具 |
| 文字工具 + 点击画布 | 在该处新建文字元素并直接进入编辑 |
| 图片工具 + 点击画布 | 插入项目里的图片（没有则弹出文件选择） |
| 「＋ 插入图片文件…」 | 复制图片进 `assets/` 并插入 |
| 拖动元素 | 移动位置 |
| 拖元素右下角 | 改尺寸 |
| 双击文字元素 | 编辑文本（Enter 或「完成」提交，「取消」放弃） |
| 双击空白处 | 就地新建文字元素 |
| 「+ 页 / − 页」 | 添加 / 删除页面（至少保留一页） |
| `Ctrl+I` → 「删除此元素」 | 删除当前选中元素 |

> 元素的 x/y/w/h 都在**页面坐标系**（默认 1280×720）里，画布负责等比缩放。
> 默认隐藏的元素用**四角括号**标出，表示"它会在录制时被快捷键触发"。

### 演练（Rehearse）

| 快捷键 / 操作 | 作用 |
| --- | --- |
| **Record 按钮 / `Ctrl+R`** | 开始录音 + 计时，画布进入「触发」状态 |
| **`1` – `9`** | 让当前页面上**第 N 个元素**（按绘制顺序）出现，并把当前录音时间戳写成 `Reveal` 事件 |
| **`Space`** | 让"下一个还没出现的元素"出现 |
| **鼠标在画布上按下拖动** | 手写；每个点都带时间戳（`t` = 已采集帧数 / 采样率） |
| `M` | 打一个标记点（`Marker` 事件） |
| `H` | 隐藏最近出现的那个元素（`Hide` 事件） |
| 点击讲稿某一句 | 记录「从此刻开始讲这一段」（`ScriptMark` 事件） |
| `←` / `→` | 上一页 / 下一页（写入 `PageChange` 事件） |
| `Esc` / `Ctrl+R` | 停止录制 |

录制时会显示带数字的**占位框**（元素出现前只显示四角括号 + 数字），
所以"按 1/2/3 会让什么出现"是看得见的，但内容不会被提前剧透。

**自动讲稿锚点**：如果某个讲稿段落标注了所属页面，而你在这页上第一次触发元素，
系统会自动把这一段记成"从这一刻开始讲"——不需要任何额外操作，
讲稿时间轴就能建立起来。

### 回放（Review）

| 操作 | 作用 |
| --- | --- |
| 播放 / 暂停按钮、`Space` | 播放与暂停音频（画面/笔迹/讲稿同步推进） |
| 拖动 / 点击时间轴 | 定位；波形上叠加了事件刻度 |
| **点击画布上的元素** | 跳到该元素**出现时**你说的那句话（`reveal_time` 反查） |
| **点击画布上的笔迹** | 跳到"当时正在写这一笔"的时间点（`nearest_stroke_time`，**线段插值**，不是取最近采样点） |
| 点击讲稿某一句 | 跳到该句的讲话时间 |
| 时间轴上的 ◀ / ▶ | 在多次演练之间切换 |

### 演讲（Present）

| 快捷键 | 作用 |
| --- | --- |
| `←` / `→` | 翻页 |
| `1` – `9` / `Space` | 触发元素出现（**不录音**） |
| `Esc` | 退出演讲模式 |

界面上只保留画布与一个「退出演讲」按钮（SPEC §22）。

---

## 4. 目录结构

```
slidetrace/
├── Cargo.toml
├── build.rs                     # slint_build::compile("ui/main.slint")
├── docs/SPEC-v1.md              # 需求（权威来源）
├── ui/                          # 全部 UI 都是 .slint，无 WebView / HTML / CSS
│   ├── theme.slint              # 主题色、共享 struct/enum、ChipButton、BracketFrame
│   ├── main.slint               # 主窗口装配：顶栏 / 页面栏 / 画布 / 讲稿 / 时间轴
│   ├── canvas.slint             # 元素渲染 + 笔迹 Path + 鼠标交互 + 文本编辑浮层
│   ├── timeline_bar.slint       # 波形 + 事件刻度 + 播放头拖动 + 演练切换
│   └── script_panel.slint       # 讲稿三档切换 + 点击跳转
├── src/
│   ├── main.rs                  # 入口、命令行参数、项目准备
│   ├── app.rs                   # 应用状态机 Create/Rehearse/Review/Present + 回调接线
│   ├── timeline.rs              # ★ state_at(t) / 命中测试 / 空间↔时间映射（纯函数 + 单测）
│   ├── storage.rs               # 项目目录读写（可注入根路径，便于单测）
│   ├── demo.rs                  # 演示项目 + 合成音频 + 占位图
│   ├── selftest.rs              # --selftest 端到端自检
│   ├── camera.rs                # 摄像头轨道的**扩展点**（V1 未实现，见 §6）
│   ├── model/                   # Project / Page / Element / Stroke / Script / Rehearsal + serde
│   │   ├── mod.rs  element.rs  page.rs  project.rs  stroke.rs  script.rs  rehearsal.rs  asset.rs
│   └── audio/
│       ├── mod.rs               # Recorder trait + WAV 读取/波形工具
│       ├── record.rs            # cpal 采集 → hound 写 WAV
│       └── play.rs              # rodio 播放 + seek + 自维护播放时钟
├── shots/                       # --screenshot 产出的四种模式截图（可删）
└── smoke_root/                  # --root ./smoke_root 的测试项目目录（可删）
```

### 数据落盘

```
%APPDATA%\slidetrace\
└── projects\<slug>\
    ├── project.json                     # 项目（页面/元素/讲稿/assets/演练元信息）
    ├── assets\figure-01.png             # 图片资源（插入时复制进来）
    └── rehearsals\<id>\
        ├── audio.wav                    # 该次演练的麦克风录音
        └── rehearsal.json               # 事件序列 + 带逐点时间的笔迹 + 音频引用
```

`project.json` 里**只存演练的元信息**（时长、事件数、是否有音频），
事件与笔迹在各自的 `rehearsal.json` 里——项目文件不会随录制次数膨胀。

---

## 5. V1 已实现能力对照 `docs/SPEC-v1.md`

| SPEC | 要求 | 状态 | 实现位置 |
| --- | --- | --- | --- |
| §0 | Rust + Slint，不用 Electron/WebView | ✅ | 全部 UI 在 `ui/*.slint`，无任何 HTML/CSS |
| §2 | `PresentationState(t)`：给定 t 返回完整演讲状态 | ✅ | `timeline.rs` → `PresentationState::state_at(t)`（纯函数，5 组单测） |
| §4 | 画布操作随时间记录（不是生成 MP4） | ✅ | `Rehearsal.events` + `Rehearsal.strokes` 落成 JSON，可继续编辑与再录 |
| §4-C | Audio / Visual / Camera 独立轨道 | ✅（Audio+Visual） | `Rehearsal.audio` 与 `events`/`strokes` 分离；Camera 见 §6 |
| §6 | 点击视觉内容 → Audio Seek | ✅ | 点击元素 → `reveal_time()` → `AudioPlayer::seek`；点击笔迹 → `nearest_stroke_time()` |
| §9-A | Waveform | ✅ | WAV 解码 → 降采样峰值 → 归一化 → 时间轴填充包络 |
| §11 | Create / Rehearse / Review / Present 四态 | ✅ | `app.rs::Mode` + 顶栏切换 |
| §12/§13 | Project 与 Rehearsal 是一等对象 | ✅ | `model/project.rs`、`model/rehearsal.rs` |
| §14 | 录制时记录麦克风/当前页/时间/元素出现/手写/标记/讲稿位置 | ✅ | 除摄像头外全部记录（`TimelineEvent` 五种 + `Stroke`） |
| §16 | 讲到哪，内容出现在哪（快捷键 + 自动绑定时间戳） | ✅ | Rehearse 模式 `1-9` / `Space` → `Reveal{element_id, t}` |
| §17 | 动画 = 时间轴上的状态变化 | ✅ | 全部表达为 `TimelineEvent`（Reveal/Hide/PageChange/Marker/ScriptMark） |
| §18 | 手写是时间事件，笔迹逐点带时间 | ✅ | `Stroke.points: Vec<{x,y,t}>`；回放时按 t 截断前缀绘制 |
| §19 | 讲稿三档 Full / Prompt / Minimal | ✅ | `ScriptSection` 三字段 + 面板切换（空档自动回退，永不空白） |
| §20 | 点击讲稿某句 → Audio Seek；播放时自动高亮当前句 | ✅ | `ScriptMark` 事件；录制时支持手动打点 + 按页面**自动打点** |
| §21 | Camera 用于演练复盘 | ❌ V1 未实现 | 见 §6，接口与数据位已预留 |
| §22/§23 | 默认界面简单，高级面板折叠 | ✅ | 默认只有顶栏/画布/讲稿/时间轴；Inspector 默认收起（`Ctrl+I`） |
| §24 | 不做模板系统/协作/云同步/剪辑器/OCR/AI | ✅ | 未实现，也未引入相关依赖 |
| §25 | V1 核心闭环 | ✅ | 由 `--selftest` 的 45 项断言端到端覆盖 |

### 关键不变量（有单测保证）

- `state_at(t)` 是**纯函数**，不依赖 Slint / cpal / rodio / 文件系统；
  Create / Rehearse / Review / Present 四个模式共用它，不存在两套可见性逻辑。
- 事件按 `t` **稳定排序**，同一时刻再按类型定序（切页 → 讲稿 → 出现 → 消失 → 标记），
  因此乱序插入后排序、以及"同一毫秒发生多个事件"的结果都是可复现的。
- 录制时间戳来自**已采集帧数 / 采样率**，不是墙上时钟，
  所以快捷键时间与 WAV 内容严格对齐，不会因为驱动缓冲区而系统性漂移。
- 文字时间戳使用 16-bit PCM WAV；播放位置由自维护的 `PlaybackClock` 推进
  （见 §6「已知限制」）。

---

## 6. 未实现项与已知限制

### 明确未实现

| 项 | 说明 |
| --- | --- |
| **摄像头录制** | SPEC §21 要求，本次**未实现**。原因：硬性约束禁止引入需要 cmake / C++ / 外部二进制的重依赖，而 Windows 上的摄像头采集方案（如 `nokhwa`）会拉入 Media Foundation 绑定与构建脚本，无法保证默认 `cargo build` 纯 Rust 可构建。**扩展点**见 `src/camera.rs`：① `Rehearsal.audio: Option<AudioRef>` 旁边预留 `video: Option<VideoRef>` 数据位，路径约定 `rehearsals/<id>/camera.mp4`；② `audio::Recorder` trait 已经把"录制源"抽象成 `elapsed()` + `finish(dir)`，摄像头只要实现同一个 trait，就能与麦克风共享同一套时间语义，上层 `app.rs` 的改动量约为 0。Inspector 面板里也明确标注了"V1 未实现"。 |
| Punch-in 局部重录 | SPEC §9-B 提到的能力，V1 未做（时间轴目前不支持编辑已有事件，只能重新录一次演练） |
| Teleprompter 自动滚动 | 讲稿面板有当前句高亮，但没有自动滚动到当前句 |
| 时间轴事件编辑 | 事件只能由录制产生，不能在回放里增删改 |
| 形状元素（箭头/高亮） | `ElementKind::Shape` 有数据位与配色，但画布上只画一个占位方框 |
| 语音转录 / 自动对齐（ASR） | SPEC §20 明确 V1 不需要；数据模型（`ScriptMark`）已为将来对齐留好位置 |
| 视频导出 | SPEC §24 排除 |

### `todo!()` / `unimplemented!()`

代码中**没有** `todo!()` 或 `unimplemented!()`。
唯一的"未实现"是一个**类型级**扩展点：`camera::CameraTrack::probe()` 恒返回 `None`
（并在单测里断言这一点），它没有 `panic`，调用方拿到的是正常的 `Option::None`。

### 已知限制 / 设计取舍

1. **无音频设备时自动降级为静音回放。** `AudioPlayer` 打不开输出设备时不会报错，
   而是把 `PlaybackClock` 单独跑起来——时间轴、画面、笔迹、讲稿同步全部照常工作，
   只是不出声。同理，没有麦克风时录制会记录一次"无声演练"（事件与笔迹照常落盘）。
   这是刻意的：产品的核心是时间轴，不该被声卡绑架。
2. **播放位置不用 `rodio::Player::get_pos()`。** rodio 的位置依赖音频线程推进，
   暂停/空队列时语义不好把握，且无设备时整条管线无法初始化。
   因此自己维护 `PlaybackClock`（`Instant` 推进），rodio 只负责出声；
   拖动时间轴时同步调用 `try_seek`，并且做 20ms 节流避免爆音。
   若某个源不支持 seek，`try_seek` 的错误会被忽略，画面定位不受影响。
3. **cpal 固定 0.17（不是 0.18）。** rodio 0.22 内部依赖 cpal 0.17；
   统一版本可以避免同一进程加载两份 WASAPI 后端。rodio 只启用
   `playback` + `hound` 两个特性（不拉 symphonia），WAV 解码与写入复用同一份 hound。
4. **文本编辑用 `LineEdit`（单行）。** Slint 1.18 的 std-widgets 没有 `TextInput`
   （只有 `LineEdit` / `TextEdit`）。元素文本目前按单行编辑；
   模型层（`ElementKind::Text { text }`）本身支持多行，UI 换 `TextEdit` 即可。
5. **画布上的元素拖动/命中测试用页面坐标。** 屏幕→页面坐标的反变换只在
   `canvas.slint` 里做一次（`mouse-x / scale`），元素、笔迹、鼠标三者的坐标系
   永远一致；缩放/letterbox 由画布内部统一处理。
6. **笔迹点做了 1.5 页面单位的抖动过滤。** 30fps 采样下高频抖动会产生大量冗余点，
   过滤后 `rehearsal.json` 体积可控，seek 精度仍由线段插值保证。
7. **`ElementKind::Image` 只支持项目内 `assets/` 与绝对路径**，没有做网络上拉取。
8. **`serial` 与 id 的一致性**：`Project::next_id` 会真正检查 id 是否已被占用，
   而不是只依赖计数器——否则手工编辑过 `project.json` 的项目会撞 id，
   进而导致删除/命中测试误伤别的元素（这个 bug 就是被 `--selftest` 抓到的）。

---

## 7. 验证记录

```bash
cargo build                     # 退出码 0，0 warning
cargo test                      # 退出码 0，66 passed / 0 failed
cargo run -- --root ./smoke_root --reset --selftest   # 退出码 0，45 项断言全部通过
cargo run -- --root ./smoke_root --screenshot shots/review.png --screenshot-mode review
```

`--selftest` 覆盖的断言（真实状态机 + 真实 Slint 模型 + 真实磁盘读写 + 真实音频时钟）：
演示项目结构、制作模式新建/编辑/删除元素、进入演练、录制中数字键与空格触发、
鼠标手写（逐点时间戳单调递增、按页面归属）、翻页、停止录制后自动落盘并进入回放、
`rehearsal.json` 往返一致、拖动时间轴定位、点击元素跳回 Reveal 时间、
点击笔迹跳回几何最近点时间、播放/暂停/回到开头、讲稿三档切换与按时间跳转、
演讲模式不录音的触发与翻页、`project.json` 保存与重新打开。

`shots/` 下是从窗口真实抓取的四种模式截图（`Window::take_snapshot()`），
用于人工确认布局；自动化断言见 `--selftest`。

---

## 8. 技术坑记录

| 坑 | 现象 | 处理 |
| --- | --- | --- |
| Slint `KeyEvent` 没有 `Key` 枚举 | 想用 `event.key == Key.LeftArrow` 编译不过 | 只有 `text` + `modifiers`；方向键是私有 Unicode 码位（`←`=`\u{F702}`、`→`=`\u{F703}`、`Esc`=`\u{1B}`、`Space`=`\u{20}`），在 `key-pressed` 里按 `text` 匹配 |
| Slint 元素 id 不能带连字符 | `label-text := Text {}` 被解析成 `label - text` | id 用驼峰；属性名（`root.canvas-pointer`）仍然可以带连字符 |
| `PointerEvent` 不带坐标 | `pointer-event(e)` 里没有 `e.position` | 坐标从 `TouchArea.mouse-x/mouse-y` 取（Slint 在分发事件**之前**就会更新它们）；按下位置另可用 `pressed-x/pressed-y` |
| 回调名与属性/枚举同名 | `callback script-level(int)` 与 `in-out property script-level` 冲突；`callback pointer(...)` 让 `mouse-cursor: pointer` 报"Callback must be called" | 回调改名（`script-level-set`、`canvas-pointer`）避开命名空间 |
| `Path.viewbox-*` 是 `float`，`length` 不能隐式转换 | `viewbox-width: root.page-w` 报错 | 显式除以 `1px` 转成纯数字 |
| `Path` 的描边在 viewbox 下会被各向异性拉伸 | 波形用描边折线画会粗细不均 | 波形改用**填充包络**（上沿 + 下沿 + `Z`），填充不受线宽缩放影响 |
| std-widgets 没有 `TextInput` | `import { TextInput }` 编译失败 | 1.18 是 `LineEdit` / `TextEdit`；`LineEdit` 不是 `FocusScope`，程序化聚焦要包一层 `FocusScope { forward-focus: editor; init => { scope.focus(); } }` |
| `ScrollView.viewport-*` 已废弃 | 编译告警 | 改用 `content-width/content-height`；同时把内层布局的宽度绑定到 ScrollView 的 id（而不是 `parent`），避免与 flickable 的内容宽度形成绑定环 |
| `ComboBox.selected` 的参数是 `string` | 与预期的 `int` 索引不符 | 干脆去掉下拉框，改用「新建 / 打开…」按钮（`rfd` 选 `project.json`），少一个组件也少一处风险 |
| rodio 0.22 API 大改 | 没有 `Sink`/`OutputStream` | 改为 `DeviceSinkBuilder::open_default_sink()` + `Player::connect_new(mixer)` + `try_seek()`；`MixerDeviceSink` 必须一直被持有，drop 即停止播放 |
| rodio 内置 WAV 解码器是 hound 后端 | `wav` 特性会拉 symphonia | 用 `features = ["playback", "hound"]`，与项目直接依赖的 hound 复用同一版本，且该后端原生支持 seek |
| cpal 0.17 的 `SampleRate` 不是 newtype | `supported.sample_rate().0` 编译失败；`device.name()` 被废弃 | 直接用 `sample_rate()`；设备名改从 `device.description().name()` 取 |
| cpal 输入流按采样格式单态化 | 想用 `build_input_stream_raw` 统一处理 | 用宏对 `f32/i16/u16/i32/i8/u8/f64` 逐个单态化，其余格式返回可读错误 |
| 无音频设备时整条播放链无法初始化 | 时间轴拖不动、画面不同步 | `AudioPlayer` 内部分离「出声」与「推进时钟」，无设备时降级为静音回放 |
| Create 模式复用 `state_at` 会带上录制痕迹 | 制作模式显示的是"最后一次演练的最后一页" | Create 模式传入空事件/空笔迹并 `show_all()`，只表达"当前页全部可见" |
| `hidden_by_default` 在 Create 模式被误判为隐藏 | 制作模式下元素整行不渲染 | `PresentationState::show_all()` 显式覆盖，隐藏语义只由画面上的四角括号提示 |

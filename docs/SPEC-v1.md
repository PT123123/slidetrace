# 演讲创作与演练工作台

## Agent 产品规格文档 · 第 1 批：产品定义、竞品分析、核心体验

---

# 0. 技术选型（不可改变）

项目固定使用：

* **Rust：核心业务逻辑**
* **Slint：全部桌面 UI**
* **Windows：第一目标平台**
* 不使用 Electron
* 不使用 WebView/HTML/CSS 作为主 UI
* 不因为任何功能方便而更换 UI 技术栈

当前 Slint Rust crate 为 1.18.0，Slint 支持通过 `.slint` 文件与 Rust 程序结合构建桌面 UI。后续实现必须以 Slint + Rust 为基础，不要将项目做成“Rust 后端 + Web 前端”。

---

# 1. 产品一句话定义

这是一个：

> **以“讲话时间”为主轴，把讲稿、视觉内容、手写、音频、摄像头以及演讲过程绑定起来的演讲创作与演练工具。**

它不是单纯的：

* PPT 制作软件
* Notability 笔记软件
* 录屏软件
* 提词器
* 视频剪辑软件

而是把这些能力组合成一个新的工作流：

```text
准备内容
   ↓
准备讲稿
   ↓
准备视觉元素
   ↓
开始演练
   ↓
同时讲话 + 操作内容 + 手写
   ↓
记录麦克风 + 摄像头 + 画布变化
   ↓
形成一次完整演练
   ↓
回放 / 复盘 / 修改
   ↓
再次演练
```

---

# 2. 产品真正的核心不是 Page，而是 Time

不要把产品核心理解成：

```text
Slide 1
Slide 2
Slide 3
```

也不要把核心理解成：

```text
Note
Audio
Camera
```

真正核心是：

```text
Time
```

在任意一个时间点：

```text
t = 32.5s
```

系统都应该知道：

```text
我说到了哪里
画面是什么状态
哪些元素已经出现
哪些元素还没有出现
手写到了哪里
摄像头正在记录什么
讲稿应该显示哪一段
```

因此整个产品应该理解为：

```text
Presentation State(t)
```

也就是：

> **时间 t 所对应的完整演讲状态。**

---

# 3. 与传统 PPT 的根本区别

普通 PPT 是：

```text
Slide 1
    ↓
Slide 2
    ↓
Slide 3
```

而本产品是：

```text
Time ────────────────────────────────→

Speech
───────●──────────●───────────────●──

Visual
       Image A      Text B          Diagram C

Handwriting
          ╱──────╲

Script
       Section 1   Section 2       Section 3
```

也就是说：

> PPT 的页面只是空间容器，不是产品的时间核心。

用户实际上是在编排：

> **“我在什么时候说什么，以及画面在这个时候应该发生什么。”**

---

# 4. 竞品提取

## 4.1 Explain Everything

这是最重要的参考对象之一。

Explain Everything 的录制系统可以记录用户在画布上的对象创建和操作过程，并把这些操作与讲解声音放到 Timeline 中。它不仅保存最终画面，而是保存“对象在什么时候发生了什么变化”。其 Timeline 同时包含视频/操作轨道与音频轨道，并支持后续编辑、混合和继续录制。

其核心启发：

```text
Canvas
+
Objects
+
Audio
+
Timeline
=
一个可回放的讲解过程
```

尤其需要借鉴：

### A. 对象操作本身应该可以被记录

例如：

```text
00:05  创建图片
00:08  移动图片
00:11  放大图片
00:15  删除图片
```

这些都可以成为时间轴上的事件。

### B. 不要把录制结果仅仅生成成视频

Explain Everything 的核心思路是：

```text
Recording
=
一组可以重新解释和编辑的操作
```

而不是：

```text
Recording
=
一个不可修改的 MP4
```

这是本产品应该重点借鉴的设计思想。

### C. Audio 与 Visual 应该独立存在

Explain Everything 将音频作为独立轨道，同时支持多个音频来源及后续编辑。

本项目也必须采用这种思想：

```text
Audio Track
Visual/Event Track
Camera Track
```

而不是把所有东西提前合并成一个视频。

---

# 5. Notability

Notability 是本产品另一个非常重要的参考对象。

它最核心的能力是：

> **音频与笔记内容同步。**

官方文档明确说明：

* 音频与 annotations 绑定
* 播放音频时点击笔记中的内容可以跳到对应录音位置
* 播放录音时新增的内容也可以同步到录音
* 可以点击文字、草图、照片，跳到当时说话的时间点

这与本项目的核心需求高度一致。

因此本产品必须继承：

```text
Visual Object
        ↕
Audio Timestamp
```

例如：

```text
Image A
   ↕
12.42s

Handwriting B
   ↕
18.73s

Text C
   ↕
25.19s
```

进一步地，本产品需要把这种能力做得比普通笔记更彻底。

---

# 6. Notability 对本项目最重要的启发：Spatial → Audio

本产品需要支持：

```text
点击视觉内容
       ↓
找到对应时间
       ↓
Audio Seek
```

例如：

```text
点击某个文字
↓
跳到 25.19s

点击某张图片
↓
跳到 32.61s

点击某条手写轨迹
↓
跳到 41.27s
```

对于手写，不应只有一个 timestamp。

例如：

```text
Stroke
P0  40.10s
P1  40.25s
P2  40.41s
P3  40.58s
P4  40.77s
```

于是：

```text
空间坐标 → 时间
```

成为一个天然映射。

---

# 7. PowerPoint

PowerPoint 值得参考的不是画布本身，而是：

> **演讲工作流。**

当前 PowerPoint 可以记录 presentation narration 和 timings，也支持在录制过程中使用 Pen / Highlighter / Eraser 等工具，并可以保存为演示文件或者视频。

因此本项目需要借鉴：

```text
Presentation
+
Narration
+
Timing
+
Presenter Workflow
```

但是不要照搬 PowerPoint 的产品结构。

PowerPoint 的核心依然是：

```text
Slide → Slide → Slide
```

本产品应该是：

```text
Speech → Time → Visual State
```

---

# 8. Prezi Video

Prezi Video 值得参考的是：

> **人 + 视觉内容 + 摄像头**

它支持录制时同时使用：

* microphone
* camera
* presentation content

并支持在录制前预览、练习，以及使用摄像头出镜。Prezi 的桌面录制流程也明确包含 narration / camera（Cameo）等录制模式。

本项目因此不能把 Camera 当成一个附属功能。

摄像头应该是一次演练数据的一部分：

```text
Rehearsal
├── Audio
├── Camera
├── Visual Timeline
├── Script
└── User Actions
```

---

# 9. SlidePunch

SlidePunch 是一个很值得参考的轻量化案例。

它把：

```text
Slides
+
Audio Waveform
+
Camera Overlay
+
Teleprompter
+
录音
+
Punch-in Audio Repair
+
Video Export
```

放在一个演讲录制工作流中。项目本身还是开源的，并且采用本地浏览器存储，不依赖后端。

对本产品的启发主要有：

### A. Waveform 很重要

录音以后，用户应该能直观看到：

```text
00:00
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
     /\      /\            /\
____/  \____/  \__________/  \____
```

因此用户可以快速定位：

* 哪里停顿
* 哪里说得很快
* 哪里重新说
* 哪里出现明显错误

### B. Punch-in Repair 很重要

如果：

```text
00:42
“今天我们讨论……呃，不对，重新来。”
```

用户不应该重新录完整段。

应该允许：

```text
选中 00:42
↓
重新录这里
↓
替换这一小段
```

这是后续非常值得实现的能力。

### C. Teleprompter

讲稿不应只是编辑内容。

录制的时候可以变成：

```text
“首先我们来看一下数据库架构……”

              ↓

自动滚动
```

---

# 10. 五个产品分别应该借什么

最终不要复制任何一个产品，而应该拆解：

```text
Explain Everything
    → 画布操作随时间记录

Notability
    → 内容 ↔ 音频时间同步

PowerPoint
    → 演讲 / 讲稿 / Presentation 工作流

Prezi Video
    → Camera + Presenter + Visual

SlidePunch
    → Waveform + Teleprompter + Rehearsal Recording
```

组合之后形成自己的产品：

```text
                    本项目

              ┌──────────────┐
              │    Script    │
              └──────┬───────┘
                     │
                     ↓
Audio ──────────── Timeline ─────────── Camera
                     │
                     ↓
                  Canvas
                     │
        ┌────────────┼────────────┐
        ↓            ↓            ↓
      Text         Image       Handwriting
        │            │            │
        └────────────┼────────────┘
                     ↓
              Rehearsal Record
```

---

# 11. 产品必须区分三个状态

产品不是永远处在“编辑 PPT”的状态。

至少应该存在：

## Create

用于：

```text
制作内容
编辑视觉元素
写讲稿
安排时间
设置动画
```

## Rehearse

用于：

```text
练习讲话
看提示
录音
录摄像头
操作画布
```

## Review

用于：

```text
回看自己
查看时间轴
听语音
看摄像头
检查讲稿
检查视觉操作
```

除此之外，再存在一个：

## Present

用于正式演讲。

这个模式应该极简，只保留：

```text
Canvas
必要提示
必要控制
```

---

# 12. 一个 Project 应该是什么

项目不要定义成：

```text
PPT 文件
```

应该定义成：

```text
Presentation Project
```

例如：

```text
“TiDB 架构分享”
```

项目下面可以包含：

```text
Content
Script
Pages / Canvas
Assets
Audio
Rehearsals
```

例如：

```text
TiDB 架构分享
│
├── Content
│   ├── Page 01
│   ├── Page 02
│   └── Page 03
│
├── Script
│
├── Assets
│   ├── architecture.png
│   └── logo.png
│
└── Rehearsals
    ├── Rehearsal 01
    ├── Rehearsal 02
    └── Rehearsal 03
```

---

# 13. Rehearsal 应该是一等对象

这是本产品非常重要的一点。

一次演练不是：

```text
一个 MP4
```

而应该是：

```text
Rehearsal
```

一次 Rehearsal 包含：

```text
Audio Recording
Camera Recording
Timeline
Script Position
Canvas Events
Element State
User Actions
```

例如：

```text
Rehearsal #07

Duration: 12:38

Audio:
██████████████████████████

Camera:
██████████████████████████

Visual Events:
   ●        ●       ●      ●

Script:
────────────●────────────────

Canvas:
00:00 → Page 01
00:17 → Image A
00:43 → Handwriting
01:12 → Diagram
...
```

---

# 14. “录制”真正记录的东西

一次正式录制期间，同时记录：

```text
Microphone
Camera
Current Page
Current Time
Element Appearance
Element Movement
Handwriting
Clicks
Markers
Script Position
```

因此最终数据不是：

```text
video.mp4
```

而是：

```text
Rehearsal
├── audio.wav
├── camera.mp4
├── timeline.json
└── event data
```

必要的时候再生成：

```text
final.mp4
```

---

# 15. 讲话应该成为整个系统的主线

用户的真实行为是：

```text
我说话
 ↓
我看到提示
 ↓
我让内容出现
 ↓
我继续说
 ↓
我手写
 ↓
我继续说
```

而不是：

```text
先做 PPT
↓
再录音
↓
再剪视频
```

所以产品交互必须优先服务于：

> **边讲边操作。**

---

# 16. 最重要的交互：说到哪里，内容出现到哪里

例如用户准备：

```text
A
B
C
D
```

开始录制：

```text
“首先我们来看 A……”
```

按一下快捷键：

```text
A → Reveal
timestamp = 08.42s
```

继续：

```text
“接下来是 B……”
```

按一下：

```text
B → Reveal
timestamp = 14.21s
```

如此形成：

```text
Audio Timeline

08.42s   A appears
14.21s   B appears
21.75s   C appears
28.32s   D appears
```

这应该成为产品最重要的核心体验之一。

---

# 17. “动画”在产品里的真正定义

不要把动画仅理解成：

```text
PowerPoint animation
```

本项目应该把它理解成：

> **某个视觉内容在时间轴上的状态变化。**

因此：

```text
文字出现
图片出现
箭头被画出来
手写被写出来
对象移动
对象消失
对象高亮
```

全部属于：

```text
Temporal Visual Event
```

用户甚至不需要主动配置复杂动画。

可以直接：

```text
讲话
↓
按快捷键
↓
元素出现
↓
自动绑定当前时间
```

---

# 18. 手写不是“绘画功能”，而是时间事件

普通手写软件：

```text
Stroke = 一条笔迹
```

本项目：

```text
Stroke =
    Geometry
    +
    Time
```

例如：

```text
Stroke #12

Point 01 → 12.01s
Point 02 → 12.08s
Point 03 → 12.13s
...
Point 87 → 13.02s
```

因此：

```text
播放 12.01s
→ 开始画

播放 12.50s
→ 画到中间

播放 13.02s
→ 完成
```

并且：

```text
点击手写位置
→ 找最近的 Stroke Point
→ Seek 到对应时间
```

这是本项目非常核心的差异化能力。

---

# 19. 讲稿不是普通 Notes

讲稿应该分三个层级：

### Full Script

完整讲话内容：

```text
首先我们来讨论整个数据库系统……
```

### Prompt

缩短成提示：

```text
数据库系统
↓
问题
↓
架构
↓
性能
```

### Minimal

只有关键词：

```text
数据库
架构
性能
```

同一个内容支持：

```text
编辑时 → Full Script
练习时 → Prompt
正式演讲 → Minimal
```

---

# 20. 讲稿应该能够和真实讲话建立联系

理想状态：

```text
Script:

“首先介绍一下整个系统的架构。”
             ↓
实际录音：
00:08.21
```

于是：

```text
点击这句话
↓
Audio Seek → 00:08.21
```

反方向：

```text
当前音频 = 00:08.21
↓
Script 自动高亮当前句
```

以后可以加入语音转录和自动对齐。

但是：

> **V1 不需要先做 AI。**

先建立正确的数据模型，让以后接 ASR / LLM 很容易。

---

# 21. Camera 的定位

Camera 不是为了制作“漂亮视频”。

它主要用于：

> **演练复盘。**

用户需要能够看到：

```text
我讲这一段的时候
我本人是什么状态
```

因此 Review 页面可以采用：

```text
┌──────────────────────────┐
│                          │
│          Camera          │
│                          │
└──────────────────────────┘

┌──────────────────────────┐
│          Canvas          │
│                          │
└──────────────────────────┘

Script:
“这里我们来看一下……”

Timeline:
──────────●────────────────
```

以后才考虑：

```text
AI 分析语速
AI 分析停顿
AI 分析 filler words
AI 分析视线
AI 分析内容覆盖
```

这些不是 V1 核心。

---

# 22. 默认界面必须非常简单

产品不能做成传统 PowerPoint 的 Ribbon。

默认用户打开以后应该看到：

```text
┌─────────────────────────────────────────────┐
│ Project                Record     Present   │
├─────────────────────────────────────────────┤
│                                             │
│                                             │
│                   Canvas                    │
│                                             │
│                                             │
├─────────────────────────────────────────────┤
│ Script                                      │
│ “首先我们来看……”                            │
├─────────────────────────────────────────────┤
│ Audio   ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~  │
└─────────────────────────────────────────────┘
```

详细设置都隐藏起来：

```text
Inspector
Advanced
Timeline
Animation
Audio
Camera
```

只有需要时展开。

---

# 23. UI 的总体原则

必须遵循：

```text
默认简单
↓
操作直接
↓
高级功能隐藏
↓
需要时展开
```

不要：

```text
所有按钮一直显示
所有参数一直显示
所有面板一直打开
```

用户最常进行的动作应该只需要很少操作：

```text
选择
放东西
写讲稿
录制
讲话
触发
回放
```

---

# 24. V1 不应该做什么

为了防止 Agent 失控，不允许第一版无限扩张。

第一阶段暂时不做：

```text
复杂 PPT 模板系统
复杂主题系统
在线协作
云端同步
多人实时编辑
完整视频剪辑器
复杂转场
3D
复杂音视频特效
OCR
高级手写识别
AI 自动生成完整演讲
AI 自动生成漂亮 PPT
```

这些以后都可以做。

第一阶段只需要证明：

> **“讲话 + 画面变化 + 手写 + 讲稿 + 音频 + 摄像头”这一整套闭环真的好用。**

---

# 25. V1 的核心用户闭环

必须优先打通下面这一条：

```text
新建项目
  ↓
创建页面
  ↓
放入文字 / 图片
  ↓
写讲稿
  ↓
点击 Record
  ↓
同时录音 + 摄像头
  ↓
开始讲话
  ↓
按快捷键让元素出现
  ↓
边讲话边手写
  ↓
结束录制
  ↓
Replay
  ↓
拖 Timeline
  ↓
视觉与音频同步
  ↓
点击某个元素
  ↓
跳到对应说话时间
  ↓
修改内容
  ↓
再次 Rehearse
```

只要这条链路成立，产品才进入正确方向。

---

# 26. 产品的最终抽象

最终把整个产品理解成：

```text
             PRESENTATION PROJECT
                     │
             ┌───────┴────────┐
             │                │
          Content          Rehearsal
             │                │
       ┌─────┼─────┐     ┌────┼─────┐
       │     │     │     │    │      │
      Text Image Stroke Audio Camera Script
       │     │     │     │    │      │
       └─────┴─────┴─────┴────┴──────┘
                     │
                  Timeline
                     │
                     ↓
              Presentation State
```

其中：

```text
Timeline = 核心
Audio = 主时间参考
Canvas = 空间表现
Script = 讲话提示
Camera = 演练记录
Elements = 视觉内容
Stroke = 带时间的视觉轨迹
Rehearsal = 一次完整过程
```

---

# 27. Agent 必须记住的一句话

在后续所有设计与实现决策中，优先遵循：

> **不要把这个项目做成“有录音功能的 PPT”，而要把它做成“以讲话时间为主轴的可交互演讲创作与演练系统”。**

第二优先级才是：

```text
它长得像不像 PPT
```

第三优先级才是：

```text
它有没有 Notability 那么多工具
```

---

# 28. 本批次结论

本项目的核心竞争力不应该来自：

```text
更强的 PPT 编辑能力
```

而应该来自：

```text
Speech
     ↕
Time
     ↕
Visual
     ↕
Interaction
```

形成完整的双向关系：

```text
我说到这里
    ↓
画面出现这个东西

我点击这个东西
    ↓
听到当时我说的话

我点击手写轨迹
    ↓
回到我当时讲话的位置

我拖动时间轴
    ↓
画面、讲稿、音频、摄像头全部同步
```

这才是整个产品最核心的产品模型。

---

# 参考竞品

Explain Everything：
https://explaineverything.com/

Explain Everything Recording：
https://help.explaineverything.com/hc/en-us/articles/360013332774-Introduction-to-Recording

Notability：
https://notability.com/

Notability Audio：
https://support.gingerlabs.com/hc/en-us/articles/206060617-Recording-and-Playing-Audio

PowerPoint Recording：
https://support.microsoft.com/en-us/powerpoint/training/record-a-presentation

Prezi Video：
https://prezi.com/video/

Prezi Video Recording：
https://support.prezi.com/hc/en-us/articles/4410541383319-Recording-with-Prezi-Video

SlidePunch：
https://github.com/bonben/slidepunch

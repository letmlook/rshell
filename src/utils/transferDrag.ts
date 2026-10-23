/**
 * 传输面板拖拽的共享状态
 *
 * 为什么不用 HTML5 drag & drop：
 * macOS 的 WKWebView 里 HTML5 拖放不成立 —— dragstart 会触发，但
 * dragover / drop 从不派发（真机合成手势实测：事件序列只有
 * dragstart → dragend，缺 drag / dragover / drop），于是投放永远不发生。
 * 同一套合成手势在 Chrome（Blink）里能完整派发，说明是 WebKit 的
 * 原生拖拽会话不被合成事件驱动，不是页面代码的问题。
 *
 * 指针事件（mousedown / mousemove / mouseup）在 WKWebView 里行为正常，
 * 于是拖拽改用指针事件实现：拖拽源自己跟踪指针，落下时按坐标找出
 * 指针下面的面板（data-drop-pane）决定投给谁。
 *
 * 状态放在模块级（与 utils/dialog 的 active 同一套做法）：两个面板是
 * 兄弟组件，拖拽中需要让「目标面板」知道有拖拽悬停。
 */
import { ref } from "vue";

/** 拖拽起点位移超过这个像素数才算拖拽（否则只是点击） */
export const DRAG_THRESHOLD = 8;

/**
 * 正在拖拽的条目：来源面板 + 条目名。
 *
 * 放在模块级（两个面板是兄弟组件）：**来源面板**与**目标面板**都要知道
 * 「现在正在拖拽、拖到哪儿了」—— 目标面板据此高亮自己。
 */
export const dragState = ref<{ from: "local" | "remote"; names: string[] } | null>(null);

/** 当前指针悬停在哪个面板上方（拖拽中有效） */
export const hoverPane = ref<"local" | "remote" | null>(null);

/**
 * 指针是否已经「抬起」，但浏览器还没派发随后的 click。
 *
 * 跨面板拖拽时，click 按 DOM 规范派发到 pointerdown / pointerup 两者的
 * **最近公共祖先**（两个面板的共同父级），不在任何单个面板里 —— 面板级
 * 的 click 拦截器看不到它，标志位就会残留，把面板内**下一个**点击也吞掉
 * （用户看到「点了没反应」）。
 *
 * 所以标志位不能只靠「拦截器看到了 click」来清：抬起后过一个 tick 就复位，
 * 标志位只用来覆盖紧跟其后的那一次 click。
 */
export const clickPending = ref(false);

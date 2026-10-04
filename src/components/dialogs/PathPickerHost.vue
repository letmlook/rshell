<script setup lang="ts">
/**
 * PathPickerHost —— 把 `utils/pathPicker.ts` 的当前请求渲染成选择窗。
 *
 * 在 App.vue 挂一次即可；具体浏览器是 LocalPathPicker。这里只做状态绑定，
 * 避免每个调用方各自维护一份「选择窗是否打开」。
 */
import LocalPathPicker from "./LocalPathPicker.vue";
import { usePathPicker } from "../../utils/pathPicker";

const { request, resolve } = usePathPicker();
</script>

<template>
  <LocalPathPicker
    v-if="request"
    :open="true"
    :mode="request.mode"
    :title="request.title"
    :initial-path="request.initialPath"
    :accept="request.accept"
    :allow-create-directory="request.allowCreateDirectory"
    @close="resolve($event)"
  />
</template>

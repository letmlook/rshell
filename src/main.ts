import { createApp } from "vue";
import { createPinia } from "pinia";
import { registerElementPlus } from "./element-plus";
import "@xterm/xterm/css/xterm.css";
import "./styles/element-plus.css";
import "./styles/tokens.css";
import "./styles/global.css";
import App from "./App.vue";

const app = createApp(App);
app.use(createPinia());
registerElementPlus(app);
app.mount("#app");
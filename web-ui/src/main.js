import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from './App.vue';
import router from './router/index.js';
import { useThemeStore } from './stores/theme.js';
import { useAuthStore } from './stores/auth.js';
import { setUnauthorizedHandler } from './lib/api.js';

import './lib/vanilla.scss';
import './lib/style.css';

const app  = createApp(App);
const pinia = createPinia();

app.use(pinia);
app.use(router);

const theme = useThemeStore();
theme.loadStored();

const auth = useAuthStore();
setUnauthorizedHandler(() => {
  auth.user = null;
  router.push('/login');
});

app.mount('#app');

const loading = document.getElementById('app-loading');
if (loading) {
  loading.style.opacity = '0';
  loading.addEventListener('transitionend', () => loading.remove(), { once: true });
}

import { createApp } from 'vue';
import './style.css';
import App from './App.vue';
import './styles/controls.css';
document.documentElement.dataset.panel = new URLSearchParams(location.search).get('panel') ?? 'main';
createApp(App).mount('#app');

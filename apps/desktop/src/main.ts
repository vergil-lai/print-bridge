import { createApp } from 'vue';
import App from './App.vue';
import { i18n } from './i18n';
import 'driver.js/dist/driver.css';
import './styles/globals.css';

createApp(App).use(i18n).mount('#app');

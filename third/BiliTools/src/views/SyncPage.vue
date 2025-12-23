<template>
  <div class="sync-page flex flex-col gap-4">
    <header class="flex items-center gap-3">
      <i :class="[$fa.weight, 'fa-wave-square text-xl']"></i>
      <div class="flex flex-col">
        <span class="text-lg font-semibold">{{ $t('syncPage.title') }}</span>
        <span class="text-sm text-(--desc-color)">{{ $t('syncPage.subtitle') }}</span>
      </div>
      <span class="badge" :class="wsConnected ? 'success' : 'muted'">
        {{ wsConnected ? $t('syncPage.serviceRunning') : $t('syncPage.serviceStopped') }}
      </span>
    </header>

    <section class="card">
      <h3 class="mb-2 text-base font-medium">{{ $t('syncPage.roomSettings') }}</h3>
      <div class="row">
        <label>{{ $t('syncPage.room') }}</label>
        <input v-model="form.room" type="text" spellcheck="false" :disabled="wsConnected" />
        <label>{{ $t('syncPage.password') }}</label>
        <input v-model="form.password" type="text" spellcheck="false" :disabled="wsConnected" />
        <button v-if="!wsConnected" class="primary" @click="connectRoom">
          <i class="fa-light fa-power-off"></i>
          <span>{{ $t('syncPage.startService') }}</span>
        </button>
        <button v-else class="ghost" @click="disconnect">
          <i class="fa-light fa-stop"></i>
          <span>{{ $t('syncPage.stopService') }}</span>
        </button>
      </div>
      <div v-if="wsConnected" class="row meta mt-2">
        <i class="fa-light fa-circle-info text-(--primary-color)"></i>
        <span>{{ $t('syncPage.mobileHint', { port: '18080' }) }}</span>
      </div>
    </section>

    <section class="card" :class="{ disabled: !wsConnected }">
      <h3 class="mb-2 text-base font-medium">{{ $t('syncPage.mediaPush') }}</h3>
      <div class="row">
        <div class="flex-1 relative">
          <input
            v-model="form.source"
            type="text"
            spellcheck="false"
            class="w-full pr-10"
            :placeholder="$t('syncPage.sourcePlaceholderMerged')"
            @keyup.enter="resolveAndPush"
          />
          <button class="absolute right-1 top-1 bottom-1 ghost !px-2" @click="chooseFile">
            <i class="fa-light fa-folder-open"></i>
          </button>
        </div>
        <button class="primary" :disabled="resolving || !wsConnected" @click="resolveAndPush">
          <i :class="['fa-light', resolving ? 'fa-spinner fa-spin' : 'fa-paper-plane']"></i>
          <span>{{ $t('syncPage.push') }}</span>
        </button>
      </div>
    </section>

    <section v-if="state.url" class="card">
      <h3 class="mb-2 text-base font-medium">{{ $t('syncPage.currentMedia') }}</h3>
      <div class="flex flex-col gap-2">
        <div class="flex items-center gap-2">
          <span class="font-medium">{{ state.title || $t('syncPage.untitled') }}</span>
          <span class="badge muted">{{ state.sourceType }}</span>
        </div>
        <div class="text-sm text-(--desc-color) break-all">
          {{ state.url }}
        </div>
        <div class="row mt-2">
          <button class="ghost" @click="pushState">
            <i class="fa-light fa-rotate"></i>
            <span>{{ $t('syncPage.resync') }}</span>
          </button>
          <span class="text-xs text-(--desc-color) ml-auto">
            {{ $t('syncPage.controlHint') }}
          </span>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { reactive, ref } from 'vue';
import * as dialog from '@tauri-apps/plugin-dialog';
import { AppLog } from '@/services/utils';
import {
  connectRoom as openSyncSocket,
  getSyncBase,
  joinRoom,
  resolveMedia,
  RoomState,
} from '@/services/sync';
import i18n from '@/i18n';

const form = reactive({
  base: getSyncBase(),
  room: 'default',
  password: '123',
  source: '',
});

const state = reactive<RoomState>({
  url: '',
  title: '',
  currentTime: 0,
  duration: 0,
  paused: true,
  playbackRate: 1,
  sourceType: '',
  updatedAt: Date.now(),
});

const tempUser = ref('');
const wsConnected = ref(false);
const resolving = ref(false);
let socket: ReturnType<typeof openSyncSocket> | null = null;

function formatErr(e: any) {
  if (!e) return 'unknown error';
  if (typeof e === 'string') return e;
  if (e.message) return e.message;
  if (e.reason) return e.reason;
  if (e instanceof Event) return e.type || 'event error';
  return String(e);
}

async function connectRoom() {
  try {
    const joined = await joinRoom(form.room, form.password, form.base);
    tempUser.value = joined.tempUser;
    socket?.close();
    socket = openSyncSocket({
      base: form.base,
      room: form.room,
      password: form.password,
      tempUser: tempUser.value,
      isHost: joined.role === 'host',
      onState: (s) => Object.assign(state, s),
      onClose: () => (wsConnected.value = false),
      onError: (e) => AppLog(formatErr(e), 'error'),
    });
    wsConnected.value = true;
    AppLog(i18n.global.t('syncPage.connectedToast'), 'success');
  } catch (e: any) {
    AppLog(formatErr(e) || 'join failed', 'error');
  }
}

function disconnect() {
  socket?.close();
  wsConnected.value = false;
}

async function chooseFile() {
  const path = await dialog.open({ directory: false });
  if (!path) return;
  form.source = Array.isArray(path) ? path[0] : (path as string);
}

async function resolveAndPush() {
  if (!wsConnected.value) {
    AppLog(i18n.global.t('syncPage.needConnect'), 'error');
    return;
  }
  if (!form.source) {
    AppLog(i18n.global.t('syncPage.needSource'), 'error');
    return;
  }
  resolving.value = true;
  try {
    const resolved = await resolveMedia(
      {
        room: form.room,
        password: form.password,
        tempUser: tempUser.value,
        path: form.source,
      },
      form.base,
    );
    state.url = resolved.url;
    state.sourceType = resolved.sourceType;
    // Use simple file name or URL as title if not parsed
    const fileName = form.source.split(/[/\\]/).pop() || form.source;
    state.title = fileName;
    state.playbackRate = 1;
    state.duration = 0; // Let client determine duration
    state.paused = true; // Start paused, let client load
    state.currentTime = 0;
    pushState();
    AppLog(i18n.global.t('syncPage.resolved', [resolved.source_type]), 'success');
  } catch (e: any) {
    AppLog(e?.message || 'resolve failed', 'error');
  } finally {
    resolving.value = false;
  }
}

function pushState() {
  if (!socket) return;
  state.updatedAt = Date.now();
  socket.sendHostUpdate({ ...state });
}
</script>

<style scoped>
@reference 'tailwindcss';
.sync-page {
  @apply h-full overflow-auto p-4 max-w-3xl mx-auto;
}
.card {
  @apply bg-(--block-color) border border-(--split-color) rounded-xl p-4 flex flex-col gap-3 transition-opacity duration-200;
}
.card.disabled {
  @apply opacity-60 pointer-events-none;
}
.row {
  @apply flex items-center gap-3 flex-wrap;
}
.row label {
  @apply text-sm text-(--desc-color) font-medium;
}
.row input {
  @apply flex-1 min-w-32 px-3 py-1.5 rounded border border-(--split-color) bg-transparent outline-none focus:border-(--primary-color) transition-colors;
}
.row button {
  @apply px-4 py-1.5 rounded border border-(--split-color) flex items-center gap-2 cursor-pointer transition-all duration-200 hover:bg-[rgba(128,128,128,0.1)];
}
.row button.primary {
  @apply bg-(--primary-color) text-(--primary-text) border-(--primary-color) hover:brightness-110;
}
.row button.ghost {
  @apply bg-transparent text-(--desc-color) border-transparent hover:bg-[rgba(128,128,128,0.1)];
}
.row button:disabled {
  @apply opacity-50 cursor-not-allowed grayscale;
}
.badge {
  @apply px-2 py-0.5 text-xs rounded-full border font-medium;
}
.badge.success {
  @apply bg-green-500/10 text-green-500 border-green-500/20;
}
.badge.muted {
  @apply bg-gray-500/10 text-gray-500 border-gray-500/20;
}
</style>

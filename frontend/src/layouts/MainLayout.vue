<template>
  <el-container class="app-shell">
    <el-aside v-if="!isMobile" :width="isCollapse ? '78px' : '248px'" class="sidebar">
      <div class="brand" :class="{ compact: isCollapse }">
        <div class="brand-mark"><span></span><span></span><span></span></div>
        <div v-if="!isCollapse" class="brand-copy">
          <strong>DNS Hearth</strong>
          <small>Network resolver</small>
        </div>
      </div>

      <div v-if="!isCollapse" class="nav-caption">工作空间</div>
      <el-menu :default-active="activeMenu" :collapse="isCollapse" router class="sidebar-menu">
        <el-menu-item v-for="item in menuItems" :key="item.path" :index="item.path">
          <el-icon><component :is="item.icon" /></el-icon>
          <template #title><span>{{ item.label }}</span></template>
        </el-menu-item>
      </el-menu>

      <div class="sidebar-footer" :class="{ compact: isCollapse }">
        <div class="status-pulse"></div>
        <div v-if="!isCollapse">
          <strong>本地管理模式</strong>
          <span>Tauri IPC 已连接</span>
        </div>
      </div>
    </el-aside>

    <el-drawer v-model="mobileMenuVisible" direction="ltr" size="260px" :with-header="false" class="mobile-drawer">
      <div class="sidebar mobile-sidebar">
        <div class="brand">
          <div class="brand-mark"><span></span><span></span><span></span></div>
          <div class="brand-copy"><strong>DNS Hearth</strong><small>Network resolver</small></div>
        </div>
        <div class="nav-caption">工作空间</div>
        <el-menu :default-active="activeMenu" router @select="mobileMenuVisible = false" class="sidebar-menu">
          <el-menu-item v-for="item in menuItems" :key="item.path" :index="item.path">
            <el-icon><component :is="item.icon" /></el-icon><span>{{ item.label }}</span>
          </el-menu-item>
        </el-menu>
      </div>
    </el-drawer>

    <el-container class="main-container">
      <el-header class="topbar">
        <div class="topbar-left">
          <button class="nav-toggle" aria-label="切换导航" @click="toggleMenu">
            <el-icon :size="18"><Expand v-if="isCollapse || isMobile" /><Fold v-else /></el-icon>
          </button>
          <div class="route-heading">
            <span>DNS Hearth</span>
            <strong>{{ currentPageTitle || '首页' }}</strong>
          </div>
        </div>
        <div class="topbar-right">
          <div class="local-chip"><span></span>本机</div>

        </div>
      </el-header>

      <el-main class="main-content">
        <div class="content-wrapper">
          <router-view v-slot="{ Component }">
            <transition name="page" mode="out-in"><component :is="Component" /></transition>
          </router-view>
        </div>
      </el-main>
    </el-container>
  </el-container>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { ElMessage } from 'element-plus'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  Odometer, Document, Edit, Connection, Coin,
  Search, List, Setting, Expand, Fold
} from '@element-plus/icons-vue'
import { useResponsive } from '../composables/useResponsive'

const route = useRoute()
const { isMobile, isCollapse } = useResponsive()
const mobileMenuVisible = ref(false)
const activeMenu = computed(() => route.path)
const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

const menuItems = [
  { path: '/', label: '首页', icon: Odometer },
  { path: '/records', label: 'DNS 记录', icon: Document },
  { path: '/rewrite', label: '重写规则', icon: Edit },
  { path: '/upstreams', label: '上游服务器', icon: Connection },
  { path: '/cache', label: '缓存管理', icon: Coin },
  { path: '/query', label: 'DNS 查询', icon: Search },
  { path: '/logs', label: '查询日志', icon: List },
  { path: '/settings', label: '系统设置', icon: Setting },
]
const pageMap = menuItems.reduce((result, item) => ({ ...result, [item.path]: item.label }), {} as Record<string, string>)
const currentPageTitle = computed(() => pageMap[route.path] || '')

function toggleMenu() {
  if (isMobile.value) mobileMenuVisible.value = !mobileMenuVisible.value
  else isCollapse.value = !isCollapse.value
}

let unlistenExitCleanupFailure: UnlistenFn | undefined
let disposed = false

onMounted(async () => {
  if (!isTauri) return
  try {
    const unlisten = await listen<string>('dns-exit-cleanup-failed', event => {
      ElMessage.error(`退出前清空 DNS 失败：${event.payload}`)
    })
    if (disposed) unlisten()
    else unlistenExitCleanupFailure = unlisten
  } catch (error) {
    ElMessage.error(`注册退出清理通知失败：${error instanceof Error ? error.message : String(error)}`)
  }
})

onUnmounted(() => {
  disposed = true
  unlistenExitCleanupFailure?.()
})
</script>

<style scoped>
.app-shell { height: 100vh; background: transparent; }
.sidebar {
  position: relative; z-index: 30; display: flex; flex-direction: column; overflow: hidden;
  border-right: 1px solid rgba(255,255,255,.07);
  background: linear-gradient(180deg, #432d2b 0%, #352423 55%, #2b1d1c 100%);
  box-shadow: 12px 0 38px rgba(8,35,40,.10); transition: width .25s ease;
}
.sidebar::before { content: ""; position: absolute; inset: 0; pointer-events: none; background: radial-gradient(circle at 20% 5%, rgba(231,139,74,.16), transparent 28%); }
.brand { position: relative; display: flex; align-items: center; height: 84px; padding: 0 22px; gap: 13px; border-bottom: 1px solid rgba(255,255,255,.07); flex-shrink: 0; }
.brand.compact { justify-content: center; padding: 0; }
.brand-mark { width: 38px; height: 38px; display: flex; align-items: center; justify-content: center; gap: 3px; border: 1px solid rgba(117,236,222,.22); border-radius: 12px; background: rgba(28,168,156,.13); box-shadow: inset 0 0 18px rgba(25,185,171,.08); }
.brand-mark span { width: 3px; border-radius: 99px; background: #ffc47c; box-shadow: 0 0 8px rgba(99,216,205,.35); }
.brand-mark span:nth-child(1) { height: 12px; }.brand-mark span:nth-child(2) { height: 22px; }.brand-mark span:nth-child(3) { height: 16px; }
.brand-copy { display: flex; flex-direction: column; min-width: 0; }
.brand-copy strong { color: #f1fbfa; font-size: 15px; font-weight: 680; letter-spacing: -.01em; }
.brand-copy small { margin-top: 3px; color: #779ca0; font-size: 10px; text-transform: uppercase; letter-spacing: .13em; }
.nav-caption { position: relative; padding: 23px 24px 8px; color: #62868a; font-size: 10px; font-weight: 700; letter-spacing: .15em; text-transform: uppercase; }
.sidebar-menu { position: relative; flex: 1; padding: 4px 11px; border: 0; background: transparent; }
.sidebar-menu :deep(.el-menu-item) { height: 45px; margin: 3px 0; padding: 0 13px !important; border-radius: 10px; color: #91adb0; font-size: 13px; font-weight: 560; }
.sidebar-menu :deep(.el-menu-item:hover) { color: #fff5ec; background: rgba(255,255,255,.055); }
.sidebar-menu :deep(.el-menu-item.is-active) { color: #fff2e5; background: linear-gradient(90deg, rgba(217,104,66,.28), rgba(217,104,66,.10)); box-shadow: inset 3px 0 0 #f3a25a; }
.sidebar-menu :deep(.el-menu-item .el-icon) { margin-right: 11px; font-size: 17px; }
.sidebar-menu.el-menu--collapse :deep(.el-menu-item) { justify-content: center; padding: 0 !important; }
.sidebar-menu.el-menu--collapse :deep(.el-menu-item .el-icon) { margin: 0; }
.sidebar-footer { position: relative; display: flex; align-items: center; gap: 10px; margin: 12px; padding: 12px; border: 1px solid rgba(115,203,195,.12); border-radius: 11px; background: rgba(255,255,255,.025); }
.sidebar-footer.compact { justify-content: center; }
.status-pulse { width: 8px; height: 8px; border-radius: 50%; background: #e3a153; box-shadow: 0 0 0 5px rgba(75,208,160,.09); }
.sidebar-footer strong, .sidebar-footer span { display: block; }.sidebar-footer strong { color: #c6dcda; font-size: 11px; }.sidebar-footer span { margin-top: 2px; color: #65868a; font-size: 10px; }
.main-container { min-width: 0; overflow: hidden; background: transparent; }
.topbar { height: 72px; display: flex; align-items: center; justify-content: space-between; padding: 0 26px; border-bottom: 1px solid rgba(205,224,222,.8); background: rgba(250,253,253,.78); backdrop-filter: blur(18px); }
.topbar-left, .topbar-right { display: flex; align-items: center; gap: 14px; }
.nav-toggle { border: 0; cursor: pointer; }
.nav-toggle { width: 36px; height: 36px; display: grid; place-items: center; border: 1px solid var(--dns-line); border-radius: 10px; color: var(--dns-primary); background: rgba(255,255,255,.8); }
.nav-toggle:hover { background: var(--dns-primary-soft); border-color: #ebc8af; }
.route-heading { display: flex; flex-direction: column; }
.route-heading span { color: var(--dns-subtle); font-size: 10px; font-weight: 650; letter-spacing: .09em; text-transform: uppercase; }
.route-heading strong { margin-top: 2px; color: var(--dns-ink); font-size: 15px; font-weight: 680; }
.local-chip { display: flex; align-items: center; gap: 7px; padding: 7px 10px; border: 1px solid #eee0d5; border-radius: 99px; color: #587174; background: rgba(255,255,255,.72); font-size: 11px; font-weight: 650; }
.local-chip span { width: 6px; height: 6px; border-radius: 50%; background: #d99145; }
.main-content { flex: 1; overflow-y: auto; padding: 28px 30px 110px; }
.content-wrapper { width: 100%; max-width: 1440px; margin: 0 auto; }
.mobile-sidebar { width: 100% !important; height: 100%; }
:deep(.mobile-drawer .el-drawer__body) { padding: 0; }
.page-enter-active, .page-leave-active { transition: opacity .16s ease, transform .16s ease; }
.page-enter-from { opacity: 0; transform: translateY(5px); }.page-leave-to { opacity: 0; transform: translateY(-3px); }
@media (max-width: 768px) {
  .topbar { height: 64px; padding: 0 14px; }.main-content { padding: 20px 16px 110px; }.hidden-xs-only { display: none !important; }.local-chip { display: none; }
}
</style>

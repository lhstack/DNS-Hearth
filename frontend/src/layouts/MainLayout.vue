<template>
  <el-container class="app-shell">
    <el-aside v-if="!isMobile" :width="isCollapse ? '78px' : '248px'" class="sidebar">
      <div class="brand" :class="{ compact: isCollapse }">
        <el-icon class="brand-mark" :size="22"><Connection /></el-icon>
        <div v-if="!isCollapse" class="brand-copy">
          <strong>DNS Hearth</strong>
          <small>Network resolver</small>
        </div>
      </div>

      <div v-if="!isCollapse" class="nav-caption">工作空间</div>
      <el-menu
        :default-active="activeMenu"
        :collapse="isCollapse"
        :collapse-transition="false"
        router
        class="sidebar-menu"
      >
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
          <el-icon class="brand-mark" :size="22"><Connection /></el-icon>
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
          <el-button
            class="nav-toggle"
            :icon="isCollapse || isMobile ? Expand : Fold"
            aria-label="切换导航"
            @click="toggleMenu"
          />
          <div class="route-heading">
            <span>DNS Hearth</span>
            <strong>{{ currentPageTitle || '首页' }}</strong>
          </div>
        </div>
        <div class="topbar-right">
          <div class="local-chip"><span></span>本机</div>

        </div>
      </el-header>

      <el-main ref="mainContentRef" class="main-content">
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
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
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
const mainContentRef = ref<{ $el?: HTMLElement } | HTMLElement | null>(null)
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

watch(
  () => route.path,
  async () => {
    await nextTick()
    const target = mainContentRef.value
    const element = target instanceof HTMLElement ? target : target?.$el
    element?.scrollTo({ top: 0, left: 0 })
  },
)

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
.app-shell { height: 100vh; background: #f5f7fa; }
.sidebar {
  position: relative; z-index: 30; display: flex; flex-direction: column; overflow: hidden;
  border-right: 1px solid #e4e7ed; background: #304156; transition: width .25s ease;
}
.brand { display: flex; align-items: center; height: 72px; padding: 0 20px; gap: 12px; border-bottom: 1px solid rgba(255,255,255,.08); flex-shrink: 0; }
.brand.compact { justify-content: center; padding: 0; }
.brand-mark { flex: 0 0 auto; color: #409eff; }
.brand.compact .brand-mark { color: #bfcbd9; }
.brand-copy { display: flex; flex-direction: column; min-width: 0; }
.brand-copy strong { color: #fff; font-size: 15px; font-weight: 600; }
.brand-copy small { margin-top: 3px; color: #aeb9c6; font-size: 10px; text-transform: uppercase; letter-spacing: .1em; }
.nav-caption { padding: 20px 22px 8px; color: #8d9bac; font-size: 11px; }
.sidebar-menu { flex: 1; padding: 4px 10px; border: 0; background: transparent; }
.sidebar-menu :deep(.el-menu-item) { height: 46px; margin: 2px 0; padding: 0 14px !important; border-radius: 4px; color: #bfcbd9; font-size: 14px; }
.sidebar-menu :deep(.el-menu-item:hover) { color: #fff; background: #263445; }
.sidebar-menu :deep(.el-menu-item.is-active) { color: #409eff; background: #263445; }
.sidebar-menu :deep(.el-menu-item .el-icon) { margin-right: 10px; font-size: 17px; }
.sidebar-menu.el-menu--collapse :deep(.el-menu-item) { justify-content: center; padding: 0 !important; }
.sidebar-menu.el-menu--collapse :deep(.el-menu-item .el-icon) { margin: 0; }
/* 折叠状态：激活项只高亮图标，不保留背景块和阴影 */
.sidebar-menu.el-menu--collapse :deep(.el-menu-item.is-active) { color: #409eff; background: transparent; box-shadow: none; }
.sidebar-menu.el-menu--collapse :deep(.el-menu-item.is-active:hover) { background: #263445; }
/* 消除 el-menu-item 折叠态默认 tooltip 触发层的残留背景 */
.sidebar-menu.el-menu--collapse :deep(.el-tooltip__trigger) { width: 100%; display: flex; justify-content: center; }
.sidebar-footer { display: flex; align-items: center; gap: 10px; margin: 12px; padding: 12px; border: 1px solid rgba(255,255,255,.10); border-radius: 4px; background: rgba(255,255,255,.04); }
.sidebar-footer.compact { justify-content: center; }
.status-pulse { width: 8px; height: 8px; border-radius: 50%; background: #67c23a; }
.sidebar-footer strong, .sidebar-footer span { display: block; }.sidebar-footer strong { color: #e4e7ed; font-size: 11px; }.sidebar-footer span { margin-top: 2px; color: #8d9bac; font-size: 10px; }
.main-container { min-width: 0; overflow: hidden; background: #f5f7fa; }
.topbar { height: 64px; display: flex; align-items: center; justify-content: space-between; padding: 0 24px; border-bottom: 1px solid #e4e7ed; background: #fff; }
.topbar-left, .topbar-right { display: flex; align-items: center; gap: 14px; }
.nav-toggle { width: 34px; height: 34px; padding: 0; color: #606266; }
.nav-toggle:hover, .nav-toggle:focus { color: #409eff; border-color: #c6e2ff; background: #ecf5ff; }
.route-heading { display: flex; flex-direction: column; }
.route-heading span { color: #909399; font-size: 10px; letter-spacing: .08em; text-transform: uppercase; }
.route-heading strong { margin-top: 2px; color: #303133; font-size: 15px; font-weight: 600; }
.local-chip { display: flex; align-items: center; gap: 7px; padding: 6px 10px; border: 1px solid #dcdfe6; border-radius: 4px; color: #606266; background: #fff; font-size: 12px; }
.local-chip span { width: 7px; height: 7px; border-radius: 50%; background: #67c23a; }
.main-content { flex: 1; overflow-y: auto; padding: 24px 28px 96px; }
.content-wrapper { width: 100%; max-width: 1440px; margin: 0 auto; }
.mobile-sidebar { width: 100% !important; height: 100%; }
:deep(.mobile-drawer .el-drawer__body) { padding: 0; }
.page-enter-active, .page-leave-active { transition: opacity .14s ease; }
.page-enter-from, .page-leave-to { opacity: 0; }
@media (max-width: 768px) {
  .topbar { height: 60px; padding: 0 14px; }.main-content { padding: 18px 14px 96px; }.hidden-xs-only { display: none !important; }.local-chip { display: none; }
}
</style>

<template>
  <div class="home-page">
    <div class="page-header">
      <div>
        <h1>首页</h1>
        <p>集中配置 UDP 本地 DNS 服务，以及每个 macOS 网络服务独立的 DNS 校正规则。</p>
      </div>
      <el-button :loading="loading" @click="loadConfiguration">
        <el-icon><Refresh /></el-icon>刷新
      </el-button>
    </div>

    <el-alert
      v-if="!isTauri"
      title="系统 DNS 与监听器配置只能在 Tauri 桌面应用中使用"
      type="warning"
      :closable="false"
      show-icon
      class="section-gap"
    />

    <el-card shadow="never" class="section-card">
      <template #header>
        <div class="card-header">
          <div>
            <div class="card-title">UDP 本地 DNS 监听</div>
            <div class="card-subtitle">作为系统 DNS 转发入口；监听 53 端口需要管理员服务。</div>
          </div>
          <el-switch v-model="udp.enabled" active-text="启用" inactive-text="停用" />
        </div>
      </template>
      <el-row :gutter="18">
        <el-col :xs="24" :sm="14">
          <el-form-item label="绑定地址">
            <el-input v-model="udp.bind_address" placeholder="127.0.0.1" />
          </el-form-item>
        </el-col>
        <el-col :xs="24" :sm="10">
          <el-form-item label="端口">
            <el-input-number v-model="udp.port" :min="1" :max="65535" style="width: 100%" />
          </el-form-item>
        </el-col>
      </el-row>
      <el-alert
        v-if="udp.enabled && udp.port === 53"
        type="info"
        :closable="false"
        show-icon
        title="首次安装特权 DNS 服务需要管理员授权；安装完成后关闭、重开监听不应再次授权。"
      />
    </el-card>

    <el-card shadow="never" class="section-card">
      <template #header>
        <div class="card-header">
          <div>
            <div class="card-title">网络服务 DNS 列表</div>
            <div class="card-subtitle">每个网络服务单独启用并配置自己的 DNS 地址，顺序会原样写入系统。</div>
          </div>
          <div class="interval-control">
            <span>检测频率</span>
            <el-input-number v-model="checkInterval" :min="1" :max="86400" controls-position="right" />
            <span>秒</span>
          </div>
        </div>
      </template>

      <el-empty v-if="!loading && services.length === 0" description="未发现可用的 macOS 网络服务" />
      <div v-loading="loading" class="service-list">
        <div v-for="service in services" :key="service.service" class="service-row">
          <div class="service-summary">
            <div>
              <div class="service-name">{{ service.service }}</div>
              <div class="current-dns">当前 DNS：{{ service.current_servers.join(', ') || '由网络自动分配' }}</div>
            </div>
            <div class="service-state">
              <el-tag v-if="service.enabled" :type="service.matches_preset ? 'success' : 'warning'" size="small">
                {{ service.matches_preset ? '符合预设' : '等待校正' }}
              </el-tag>
              <el-switch v-model="service.enabled" active-text="监听" inactive-text="忽略" />
              <el-button
                v-if="!service.enabled"
                size="small"
                text
                type="warning"
                :loading="clearingService === service.service"
                @click="clearDns(service)"
              >
                <el-icon><Delete /></el-icon>清空 DNS
              </el-button>
            </div>
          </div>
          <el-select
            v-model="service.preset_servers"
            multiple
            allow-create
            filterable
            default-first-option
            placeholder="输入该网络服务的 DNS IP，按回车添加"
            style="width: 100%"
          >
            <el-option v-for="server in service.preset_servers" :key="server" :label="server" :value="server" />
          </el-select>
          <div class="quick-actions">
            <el-button size="small" text type="primary" @click="prependLocalDns(service)">加入本地 DNS 127.0.0.1</el-button>
            <el-button size="small" text @click="useCurrentDns(service)">使用当前 DNS</el-button>
          </div>
        </div>
      </div>
    </el-card>

    <div class="apply-status" role="status" aria-live="polite">
      <span :class="['apply-dot', { active: saving, error: applyError }]" />
      <span>{{ applyError || (saving ? saveStage : '配置会自动生效') }}</span>
      <el-button v-if="saving" size="small" text @click="cancelSave">取消</el-button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Delete, Refresh } from '@element-plus/icons-vue'
import api from '../api'
import {
  applyHomeConfiguration,
  clearNetworkServiceDns,
  getNetworkServices,
  getSystemDnsConfig,
  type NetworkServiceDns,
} from '../api/systemDns'

interface Listener {
  protocol: string
  enabled: boolean
  bind_address: string
  port: number
}

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
const loading = ref(false)
const hydrating = ref(false)
const saving = ref(false)
const saveStage = ref('正在应用…')
const applyError = ref('')
let saveAttempt = 0
let applyTimer: number | undefined
const checkInterval = ref(30)
const services = ref<NetworkServiceDns[]>([])
const clearingService = ref('')
const udp = reactive({ enabled: false, bind_address: '127.0.0.1', port: 53 })

function messageOf(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  if (error && typeof error === 'object' && 'message' in error) return String((error as { message: unknown }).message)
  return '操作失败'
}

async function loadConfiguration() {
  if (!isTauri) return
  hydrating.value = true
  loading.value = true
  try {
    const [config, serviceStatus, listenerResponse] = await Promise.all([
      getSystemDnsConfig(),
      getNetworkServices(),
      api.get<{ data: Listener[] }>('/api/listeners'),
    ])
    checkInterval.value = config.check_interval_secs
    services.value = serviceStatus
    const listener = listenerResponse.data.data.find(item => item.protocol === 'udp')
    if (!listener) throw new Error('UDP 监听器配置不存在')
    Object.assign(udp, listener)
  } catch (error) {
    ElMessage.error(messageOf(error))
  } finally {
    loading.value = false
    await nextTick()
    hydrating.value = false
  }
}

function prependLocalDns(service: NetworkServiceDns) {
  service.preset_servers = ['127.0.0.1', ...service.preset_servers.filter(item => item !== '127.0.0.1')]
  service.enabled = true
}

function useCurrentDns(service: NetworkServiceDns) {
  service.preset_servers = [...service.current_servers]
  service.enabled = true
}

async function clearDns(service: NetworkServiceDns) {
  try {
    await ElMessageBox.confirm(
      `确定清空“${service.service}”当前已设置的系统 DNS 吗？这不会删除保存的 DNS 预设。`,
      '清空系统 DNS',
      { confirmButtonText: '清空', cancelButtonText: '取消', type: 'warning' },
    )
  } catch (error) {
    if (error !== 'cancel') ElMessage.error(messageOf(error))
    return
  }

  clearingService.value = service.service
  try {
    const cleared = await clearNetworkServiceDns(service.service)
    Object.assign(service, cleared)
    ElMessage.success(`已清空“${service.service}”的系统 DNS`)
  } catch (error) {
    ElMessage.error(messageOf(error))
  } finally {
    clearingService.value = ''
  }
}

function buildBindings() {
  return services.value.map(service => ({
    service: service.service,
    enabled: service.enabled,
    preset_servers: service.preset_servers.map(item => item.trim()).filter(Boolean),
  }))
}

function scheduleApply() {
  if (!isTauri || loading.value || hydrating.value) return
  if (applyTimer !== undefined) window.clearTimeout(applyTimer)
  applyTimer = window.setTimeout(() => {
    applyTimer = undefined
    void applyConfiguration()
  }, 500)
}

async function applyConfiguration() {
  const attempt = ++saveAttempt
  saving.value = true
  applyError.value = ''
  saveStage.value = udp.enabled && udp.port === 53 ? '正在启动 DNS 服务…' : '正在应用配置…'
  try {
    const result = await Promise.race([
      applyHomeConfiguration({
        system_dns: { bindings: buildBindings(), check_interval_secs: checkInterval.value },
        udp_enabled: udp.enabled,
        udp_bind_address: udp.bind_address.trim(),
        udp_port: udp.port,
      }),
      new Promise<never>((_, reject) => window.setTimeout(
        () => reject(new Error('应用配置超过 40 秒，已停止等待。请检查 macOS 管理员授权窗口或特权 DNS 服务状态。')),
        40_000,
      )),
    ])
    if (attempt !== saveAttempt) return
    hydrating.value = true
    try {
      services.value = result.services
      Object.assign(udp, result.udp_listener)
      await nextTick()
    } finally {
      hydrating.value = false
    }
    ElMessage.success('配置已自动应用')
  } catch (error) {
    if (attempt !== saveAttempt) return
    applyError.value = messageOf(error)
    ElMessage.error(applyError.value)
  } finally {
    if (attempt === saveAttempt) saving.value = false
  }
}

function cancelSave() {
  saveAttempt += 1
  if (applyTimer !== undefined) {
    window.clearTimeout(applyTimer)
    applyTimer = undefined
  }
  saving.value = false
  saveStage.value = '正在应用…'
  ElMessage.info('已取消等待中的自动应用')
}

onMounted(() => {
  void loadConfiguration()
})

watch([checkInterval, () => udp.enabled, () => udp.bind_address, () => udp.port], scheduleApply)
watch(
  () => services.value.map(service => `${service.service}|${service.enabled}|${service.preset_servers.join(',')}`).join('\n'),
  scheduleApply,
)

onUnmounted(() => {
  if (applyTimer !== undefined) window.clearTimeout(applyTimer)
})

</script>

<style scoped>
.home-page { max-width: 1180px; margin: 0 auto; padding-bottom: 96px; }
.page-header, .card-header, .service-summary { display: flex; justify-content: space-between; align-items: center; gap: 18px; }
.page-header { margin-bottom: 22px; }
h1 { margin: 0 0 8px; font-size: 28px; }
.page-header p, .card-subtitle, .current-dns { margin: 0; color: var(--el-text-color-secondary); }
.section-card { margin-bottom: 20px; border-radius: 12px; }
.section-gap { margin-bottom: 20px; }
.card-title { font-size: 17px; font-weight: 650; margin-bottom: 5px; }
.interval-control { display: flex; align-items: center; gap: 8px; white-space: nowrap; }
.service-list { display: grid; gap: 14px; }
.service-row { border: 1px solid var(--el-border-color-lighter); border-radius: 10px; padding: 16px; background: var(--el-fill-color-blank); }
.service-summary { margin-bottom: 12px; }
.service-name { font-weight: 650; font-size: 16px; margin-bottom: 4px; }
.current-dns { font-size: 13px; }
.service-state, .quick-actions { display: flex; align-items: center; gap: 10px; }
.quick-actions { margin-top: 6px; }
.apply-status { position: fixed; z-index: 20; left: 260px; right: 0; bottom: 0; display: flex; align-items: center; justify-content: center; gap: 9px; padding: 10px 24px; color: var(--el-text-color-secondary); font-size: 12px; background: rgba(255,255,255,.94); border-top: 1px solid var(--el-border-color-light); backdrop-filter: blur(12px); }
.apply-dot { width: 7px; height: 7px; border-radius: 50%; background: #9aa7a5; }
.apply-dot.active { background: #d96842; box-shadow: 0 0 0 4px rgba(217,104,66,.12); }
.apply-dot.error { background: #d94f4f; }
@media (max-width: 767px) {
  .page-header, .card-header, .service-summary { align-items: flex-start; flex-direction: column; }
  .service-state { width: 100%; justify-content: space-between; }
  .apply-status { left: 0; }
}
</style>

<style scoped>
.home-page { max-width: 1160px; }
.page-header { align-items: flex-start; }
.section-card { border-radius: 16px !important; }
.section-card :deep(.el-card__header) { background: linear-gradient(90deg, rgba(230,246,244,.64), rgba(255,255,255,.18)); }
.service-list { gap: 12px; }
.service-row { position: relative; overflow: hidden; border-color: #eee0d5; border-radius: 12px; background: linear-gradient(135deg, rgba(255,255,255,.98), rgba(246,251,250,.94)); transition: border-color .2s ease, transform .2s ease, box-shadow .2s ease; }
.service-row::before { content: ""; position: absolute; inset: 0 auto 0 0; width: 3px; background: #efd0b8; }
.service-row:hover { transform: translateY(-1px); border-color: #ebc8af; box-shadow: 0 12px 28px rgba(17,75,76,.07); }
.service-name { color: #442d2a; letter-spacing: -.015em; }
.interval-control { padding: 7px 10px; border: 1px solid #eee0d5; border-radius: 10px; color: #5b737b; background: rgba(255,255,255,.75); }
.apply-status { left: 248px; border-top-color: #ead8ca; background: rgba(247,251,250,.92); box-shadow: 0 -12px 36px rgba(17,55,60,.06); }
@media (max-width: 767px) { .apply-status { left: 0; } }
</style>

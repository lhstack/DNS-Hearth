<template>
  <div class="settings">
    <!-- 页面标题 -->
    <div class="page-header">
      <div class="header-left">
        <h1>系统设置</h1>
        <p class="subtitle">配置查询策略与系统参数</p>
      </div>
      <el-button type="primary" @click="refreshAll" class="action-btn">
        <el-icon><Refresh /></el-icon>
        <span class="hidden-xs-only">刷新全部</span>
      </el-button>
    </div>

    <!-- 统计卡片 -->
    <el-row :gutter="20" class="stats-row">
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: #409eff;">
            <el-icon><Timer /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ formatUptimeShort(status.uptime_seconds) }}</span>
            <span class="stat-label">运行时间</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: #67c23a;">
            <el-icon><DataAnalysis /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ status.query?.total_queries || 0 }}</span>
            <span class="stat-label">总查询</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: #f56c6c;">
            <el-icon><Box /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ status.cache?.entries || 0 }}</span>
            <span class="stat-label">缓存条目</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: #e6a23c;">
            <el-icon><Connection /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ status.upstreams?.healthy || 0 }}/{{ status.upstreams?.total || 0 }}</span>
            <span class="stat-label">健康上游</span>
          </div>
        </div>
      </el-col>
    </el-row>

    <el-row :gutter="20" class="equal-height-row">
      <!-- 查询策略 -->
      <el-col :xs="24" :md="12">
        <el-card class="strategy-card" shadow="never">
          <template #header>
            <div class="card-title">
              <el-icon><Setting /></el-icon>
              <span>查询策略</span>
            </div>
          </template>
          <div v-loading="loadingStrategy">
            <div class="current-strategy">
              <span class="current-label">当前策略</span>
              <el-tag type="primary" size="large" effect="dark">
                {{ getStrategyLabel(currentStrategy.strategy) }}
              </el-tag>
            </div>
            <p class="strategy-desc">{{ currentStrategy.description }}</p>

            <el-divider />

            <div class="strategy-options">
              <el-radio-group v-model="selectedStrategy" class="strategy-radio-group">
                <div
                  v-for="strategy in availableStrategies"
                  :key="strategy.name"
                  class="strategy-item"
                  :class="{ 'is-selected': selectedStrategy === strategy.name }"
                  @click="selectedStrategy = strategy.name"
                >
                  <el-radio :value="strategy.name" />
                  <div class="strategy-content">
                    <span class="strategy-name">{{ getStrategyLabel(strategy.name) }}</span>
                    <span class="strategy-option-desc">{{ strategy.description }}</span>
                  </div>
                </div>
              </el-radio-group>
            </div>

            <el-button
              type="primary"
              size="large"
              @click="saveStrategy"
              :loading="savingStrategy"
              :disabled="selectedStrategy === currentStrategy.strategy"
              class="save-btn primary-action"
            >
              <el-icon><Check /></el-icon>
              保存策略
            </el-button>
          </div>
        </el-card>
      </el-col>

      <!-- 记录类型开关 -->
      <el-col :xs="24" :md="12">
        <el-card class="record-types-card" shadow="never">
          <template #header>
            <div class="card-header">
              <div class="card-title">
                <el-icon><Switch /></el-icon>
                <span>记录类型开关</span>
              </div>
              <el-button type="primary" link @click="fetchSettings" :loading="loadingSettings">
                <el-icon><Refresh /></el-icon>
                刷新
              </el-button>
            </div>
          </template>
          <div v-loading="loadingSettings">
            <p class="section-desc">关闭某些记录类型后，对应的 DNS 查询将返回 NXDOMAIN</p>
            <div class="record-type-list">
              <div
                v-for="recordType in recordTypes"
                :key="recordType.type"
                class="record-type-item"
              >
                <div class="record-type-info">
                  <span class="record-type-name">{{ recordType.type }}</span>
                  <span class="record-type-desc">{{ recordType.description }}</span>
                </div>
                <el-switch
                  v-model="recordType.enabled"
                  @change="saveRecordTypeSettings"
                  :loading="savingSettings"
                  inline-prompt
                  active-text="开"
                  inactive-text="关"
                />
              </div>
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 日志管理 -->
    <el-row :gutter="20" style="margin-top: 20px;">
      <el-col :span="24">
        <el-card class="log-cleanup-card" shadow="never">
          <template #header>
            <div class="card-header">
              <div class="card-title">
                <el-icon><Delete /></el-icon>
                <span>查询日志管理</span>
              </div>
              <el-button type="primary" link @click="fetchRetentionSettings" :loading="loadingRetention">
                <el-icon><Refresh /></el-icon>
                刷新
              </el-button>
            </div>
          </template>
          <div v-loading="loadingRetention">
            <el-row :gutter="24">
              <!-- 手动清理 -->
              <el-col :xs="24" :md="12">
                <div class="cleanup-section">
                  <h4 class="section-title">手动清理</h4>
                  <p class="section-desc">选择日期，清理该日期之前的所有查询日志</p>
                  <div class="cleanup-form">
                    <el-date-picker
                      v-model="cleanupBeforeDate"
                      type="date"
                      placeholder="选择日期"
                      format="YYYY-MM-DD"
                      value-format="YYYY-MM-DD"
                      :disabled-date="disabledDate"
                      class="date-picker-responsive"
                    />
                    <div class="cleanup-btns">
                      <el-button
                        type="warning"
                        @click="cleanupBeforeDateAction"
                        :loading="cleaningLogs"
                        :disabled="!cleanupBeforeDate"
                      >
                        <el-icon><Delete /></el-icon>
                        清理日志
                      </el-button>
                      <el-button
                        type="danger"
                        @click="cleanupAllLogsAction"
                        :loading="cleaningLogs"
                      >
                        <el-icon><DeleteFilled /></el-icon>
                        清空全部
                      </el-button>
                    </div>
                  </div>
                  <div class="oldest-log-info" v-if="retentionSettings.oldest_log_date">
                    <el-icon><InfoFilled /></el-icon>
                    <span>最早日志时间: {{ formatDate(retentionSettings.oldest_log_date) }}</span>
                  </div>
                </div>
              </el-col>
              <!-- 自动清理 -->
              <el-col :xs="24" :md="12">
                <div class="cleanup-section">
                  <h4 class="section-title">自动清理</h4>
                  <p class="section-desc">启用后，系统将自动清理超过保留天数的日志</p>
                  <div class="auto-cleanup-form">
                    <div class="auto-cleanup-toggle">
                      <span class="toggle-label">启用自动清理</span>
                      <el-switch
                        v-model="retentionSettings.auto_cleanup_enabled"
                        @change="saveRetentionSettings"
                        :loading="savingRetention"
                        inline-prompt
                        active-text="开"
                        inactive-text="关"
                      />
                    </div>
                    <div class="retention-days-input">
                      <span class="input-label">保留天数</span>
                      <el-input-number
                        v-model="retentionSettings.retention_days"
                        :min="1"
                        :max="365"
                        :disabled="!retentionSettings.auto_cleanup_enabled"
                        @change="saveRetentionSettings"
                        class="number-input-responsive"
                      />
                      <span class="input-suffix">天</span>
                    </div>
                  </div>
                </div>
              </el-col>
            </el-row>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 告警与状态 -->
    <el-row :gutter="20" style="margin-top: 20px;" class="equal-height-row">
      <!-- 系统状态 -->
      <el-col :span="24">
        <el-card class="status-card" shadow="never">
          <template #header>
            <div class="card-header">
              <div class="card-title">
                <el-icon><Monitor /></el-icon>
                <span>系统状态</span>
              </div>
              <el-button type="primary" link @click="fetchStatus">
                <el-icon><Refresh /></el-icon>
                刷新
              </el-button>
            </div>
          </template>
          <div v-loading="loadingStatus" class="status-content">
            <div class="status-item">
              <span class="status-label">运行状态</span>
              <el-tag :type="status.status === 'running' ? 'success' : 'danger'" effect="dark">
                {{ status.status === 'running' ? '运行中' : '异常' }}
              </el-tag>
            </div>
            <div class="status-item">
              <span class="status-label">运行时间</span>
              <span class="status-value">{{ formatUptime(status.uptime_seconds) }}</span>
            </div>
            <div class="status-item">
              <span class="status-label">缓存命中率</span>
              <span class="status-value highlight">{{ ((status.cache?.hit_rate || 0) * 100).toFixed(1) }}%</span>
            </div>
            <div class="status-item">
              <span class="status-label">今日查询</span>
              <span class="status-value">{{ status.query?.queries_today || 0 }}</span>
            </div>
            <div class="status-item">
              <span class="status-label">上游服务器</span>
              <span class="status-value">
                <el-tag type="success" size="small">{{ status.upstreams?.healthy || 0 }} 健康</el-tag>
                <el-tag type="info" size="small" style="margin-left: 4px;">{{ status.upstreams?.total || 0 }} 总计</el-tag>
              </span>
            </div>
            <div class="status-item">
              <span class="status-label">查询策略</span>
              <span class="status-value">{{ getStrategyLabel(status.strategy) }}</span>
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <el-row :gutter="20" style="margin-top: 20px;">
      <!-- 健康检查 -->
      <el-col :span="24">
        <el-card class="health-card" shadow="never">
          <template #header>
            <div class="card-header">
              <div class="card-title">
                <el-icon><FirstAidKit /></el-icon>
                <span>健康检查</span>
              </div>
              <el-button type="primary" link @click="fetchHealth">
                <el-icon><Refresh /></el-icon>
                检查
              </el-button>
            </div>
          </template>
          <div v-loading="loadingHealth">
            <div class="health-list">
              <div class="health-item" :class="{ 'is-healthy': health.database }">
                <div class="health-icon" :style="{ background: health.database ? '#67c23a' : '#f56c6c' }">
                  <el-icon><Coin /></el-icon>
                </div>
                <div class="health-info">
                  <span class="health-label">数据库</span>
                  <span class="health-status" :class="{ 'is-ok': health.database }">
                    {{ health.database ? '正常' : '异常' }}
                  </span>
                </div>
                <el-icon class="health-check-icon" :class="{ 'is-ok': health.database }">
                  <CircleCheck v-if="health.database" />
                  <CircleClose v-else />
                </el-icon>
              </div>
              <div class="health-item" :class="{ 'is-healthy': health.cache }">
                <div class="health-icon" :style="{ background: health.cache ? '#67c23a' : '#f56c6c' }">
                  <el-icon><Box /></el-icon>
                </div>
                <div class="health-info">
                  <span class="health-label">缓存</span>
                  <span class="health-status" :class="{ 'is-ok': health.cache }">
                    {{ health.cache ? '正常' : '异常' }}
                  </span>
                </div>
                <el-icon class="health-check-icon" :class="{ 'is-ok': health.cache }">
                  <CircleCheck v-if="health.cache" />
                  <CircleClose v-else />
                </el-icon>
              </div>
              <div class="health-item" :class="{ 'is-healthy': health.upstreams }">
                <div class="health-icon" :style="{ background: health.upstreams ? '#67c23a' : '#f56c6c' }">
                  <el-icon><Connection /></el-icon>
                </div>
                <div class="health-info">
                  <span class="health-label">上游服务器</span>
                  <span class="health-status" :class="{ 'is-ok': health.upstreams }">
                    {{ health.upstreams ? '正常' : '无可用' }}
                  </span>
                </div>
                <el-icon class="health-check-icon" :class="{ 'is-ok': health.upstreams }">
                  <CircleCheck v-if="health.upstreams" />
                  <CircleClose v-else />
                </el-icon>
              </div>
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  Refresh, Timer, DataAnalysis, Box, Connection, Setting, Check,
  Monitor, FirstAidKit, Coin, CircleCheck, CircleClose, Switch,
  Delete, DeleteFilled, InfoFilled
} from '@element-plus/icons-vue'
import api from '../api'

interface Strategy {
  strategy: string
  description: string
}

interface StrategyInfo {
  name: string
  description: string
}

interface SystemStatus {
  status: string
  uptime_seconds: number
  cache: {
    entries: number
    hits: number
    misses: number
    hit_rate: number
    default_ttl: number
    max_entries: number
  }
  query: {
    total_queries: number
    cache_hits: number
    queries_today: number
  }
  upstreams: {
    total: number
    healthy: number
    servers: any[]
  }
  strategy: string
}

interface HealthCheck {
  status: string
  database: boolean
  cache: boolean
  upstreams: boolean
}

const currentStrategy = ref<Strategy>({
  strategy: '',
  description: ''
})
const selectedStrategy = ref('')
const availableStrategies = ref<StrategyInfo[]>([])
const loadingStrategy = ref(false)
const savingStrategy = ref(false)

const status = ref<SystemStatus>({
  status: '',
  uptime_seconds: 0,
  cache: { entries: 0, hits: 0, misses: 0, hit_rate: 0, default_ttl: 60, max_entries: 10000 },
  query: { total_queries: 0, cache_hits: 0, queries_today: 0 },
  upstreams: { total: 0, healthy: 0, servers: [] },
  strategy: ''
})
const loadingStatus = ref(false)

const health = ref<HealthCheck>({
  status: '',
  database: false,
  cache: false,
  upstreams: false
})
const loadingHealth = ref(false)

// Record type settings
interface RecordTypeItem {
  type: string
  description: string
  enabled: boolean
}

const recordTypes = ref<RecordTypeItem[]>([
  { type: 'A', description: 'IPv4 地址记录', enabled: true },
  { type: 'AAAA', description: 'IPv6 地址记录', enabled: true },
  { type: 'CNAME', description: '别名记录', enabled: true },
  { type: 'MX', description: '邮件交换记录', enabled: true },
  { type: 'TXT', description: '文本记录', enabled: true },
  { type: 'PTR', description: '反向解析记录', enabled: true },
  { type: 'NS', description: '域名服务器记录', enabled: true },
])
const loadingSettings = ref(false)
const savingSettings = ref(false)
let saveSettingsTimer: ReturnType<typeof setTimeout> | null = null

// Log retention settings
interface RetentionSettings {
  auto_cleanup_enabled: boolean
  retention_days: number
  oldest_log_date: string | null
}

const retentionSettings = ref<RetentionSettings>({
  auto_cleanup_enabled: false,
  retention_days: 30,
  oldest_log_date: null
})
const loadingRetention = ref(false)
const savingRetention = ref(false)
const cleaningLogs = ref(false)
const cleanupBeforeDate = ref<string>('')
let saveRetentionTimer: ReturnType<typeof setTimeout> | null = null

const strategyLabels: Record<string, string> = {
  concurrent: '并发查询',
  fastest: '最快响应',
  round_robin: '轮询',
  random: '随机'
}

function getStrategyLabel(strategy: string): string {
  return strategyLabels[strategy] || strategy
}

function formatUptime(seconds: number): string {
  if (!seconds) return '-'
  const days = Math.floor(seconds / 86400)
  const hours = Math.floor((seconds % 86400) / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  const secs = seconds % 60

  const parts = []
  if (days > 0) parts.push(`${days}天`)
  if (hours > 0) parts.push(`${hours}小时`)
  if (minutes > 0) parts.push(`${minutes}分钟`)
  if (secs > 0 || parts.length === 0) parts.push(`${secs}秒`)

  return parts.join(' ')
}

function formatUptimeShort(seconds: number): string {
  if (!seconds) return '-'
  const days = Math.floor(seconds / 86400)
  const hours = Math.floor((seconds % 86400) / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)

  if (days > 0) return `${days}d ${hours}h`
  if (hours > 0) return `${hours}h ${minutes}m`
  return `${minutes}m`
}

function refreshAll() {
  fetchStrategy()
  fetchStatus()
  fetchHealth()
  fetchSettings()
  fetchRetentionSettings()
}

async function fetchSettings() {
  loadingSettings.value = true
  try {
    const response = await api.get('/api/settings')
    const disabledTypes = response.data.disabled_record_types || []
    // Update enabled status based on disabled types
    recordTypes.value.forEach(rt => {
      rt.enabled = !disabledTypes.includes(rt.type)
    })
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '获取设置失败')
  } finally {
    loadingSettings.value = false
  }
}

async function saveRecordTypeSettings() {
  // 防抖：快速切换多个开关时，只在最后一次操作后保存
  if (saveSettingsTimer) {
    clearTimeout(saveSettingsTimer)
  }

  saveSettingsTimer = setTimeout(async () => {
    savingSettings.value = true
    try {
      const disabledTypes = recordTypes.value
        .filter(rt => !rt.enabled)
        .map(rt => rt.type)

      await api.put('/api/settings', {
        disabled_record_types: disabledTypes
      })
      ElMessage.success('设置已保存')
    } catch (error: any) {
      ElMessage.error(error.response?.data?.message || '保存设置失败')
      // Revert on error
      fetchSettings()
    } finally {
      savingSettings.value = false
    }
  }, 500)
}

async function fetchStrategy() {
  loadingStrategy.value = true
  try {
    const [currentRes, availableRes] = await Promise.all([
      api.get('/api/strategy'),
      api.get('/api/strategy/available')
    ])
    currentStrategy.value = currentRes.data
    selectedStrategy.value = currentRes.data.strategy
    availableStrategies.value = availableRes.data.strategies
    if (!currentStrategy.value.strategy || availableStrategies.value.length === 0) {
      throw new Error('查询策略接口返回了不完整的数据')
    }
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '获取策略配置失败')
  } finally {
    loadingStrategy.value = false
  }
}

async function saveStrategy() {
  savingStrategy.value = true
  try {
    const response = await api.put('/api/strategy', { strategy: selectedStrategy.value })
    currentStrategy.value = response.data
    ElMessage.success('策略已更新')
    fetchStatus()
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '保存策略失败')
  } finally {
    savingStrategy.value = false
  }
}

async function fetchStatus() {
  loadingStatus.value = true
  try {
    const response = await api.get('/api/status')
    status.value = response.data
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '获取系统状态失败')
  } finally {
    loadingStatus.value = false
  }
}

async function fetchHealth() {
  loadingHealth.value = true
  try {
    const response = await api.get('/api/status/health')
    health.value = response.data
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '健康检查失败')
  } finally {
    loadingHealth.value = false
  }
}

// Log retention methods
async function fetchRetentionSettings() {
  loadingRetention.value = true
  try {
    const response = await api.get('/api/logs/retention')
    retentionSettings.value = response.data
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '获取日志保留设置失败')
  } finally {
    loadingRetention.value = false
  }
}

async function saveRetentionSettings() {
  // 防抖
  if (saveRetentionTimer) {
    clearTimeout(saveRetentionTimer)
  }

  saveRetentionTimer = setTimeout(async () => {
    savingRetention.value = true
    try {
      await api.put('/api/logs/retention', {
        auto_cleanup_enabled: retentionSettings.value.auto_cleanup_enabled,
        retention_days: retentionSettings.value.retention_days
      })
      ElMessage.success('日志保留设置已保存')
    } catch (error: any) {
      ElMessage.error(error.response?.data?.message || '保存设置失败')
      fetchRetentionSettings()
    } finally {
      savingRetention.value = false
    }
  }, 500)
}

async function cleanupBeforeDateAction() {
  if (!cleanupBeforeDate.value) return

  try {
    await ElMessageBox.confirm(
      `确定要删除 ${cleanupBeforeDate.value} 之前的所有查询日志吗？此操作不可恢复。`,
      '确认清理',
      {
        confirmButtonText: '确定清理',
        cancelButtonText: '取消',
        type: 'warning'
      }
    )

    cleaningLogs.value = true
    const response = await api.delete('/api/logs/cleanup/before', {
      params: { before_date: cleanupBeforeDate.value }
    })
    ElMessage.success(`已删除 ${response.data.deleted_count} 条日志`)
    cleanupBeforeDate.value = ''
    fetchRetentionSettings()
  } catch (error: any) {
    if (error !== 'cancel') {
      ElMessage.error(error.response?.data?.message || '清理日志失败')
    }
  } finally {
    cleaningLogs.value = false
  }
}

async function cleanupAllLogsAction() {
  try {
    await ElMessageBox.confirm(
      '确定要删除所有查询日志吗？此操作不可恢复！',
      '确认清空',
      {
        confirmButtonText: '确定清空',
        cancelButtonText: '取消',
        type: 'error'
      }
    )

    cleaningLogs.value = true
    const response = await api.delete('/api/logs/cleanup/all')
    ElMessage.success(`已删除 ${response.data.deleted_count} 条日志`)
    fetchRetentionSettings()
  } catch (error: any) {
    if (error !== 'cancel') {
      ElMessage.error(error.response?.data?.message || '清空日志失败')
    }
  } finally {
    cleaningLogs.value = false
  }
}

function disabledDate(time: Date) {
  return time.getTime() > Date.now()
}

function formatDate(dateStr: string | null): string {
  if (!dateStr) return '-'
  const date = new Date(dateStr)
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  })
}

onMounted(() => {
  fetchStrategy()
  fetchStatus()
  fetchHealth()
  fetchSettings()
  fetchRetentionSettings()
})
</script>

<style scoped>
.settings {
  max-width: 1400px;
  margin: 0 auto;
}

/* 页面标题 */
.page-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: 24px;
}

.header-left h1 {
  margin: 0 0 8px 0;
  font-size: 24px;
  font-weight: 600;
  color: #303133;
}

.subtitle {
  margin: 0;
  font-size: 14px;
  color: #909399;
}

/* 统计卡片 */
.stats-row {
  margin-bottom: 24px;
}

.stat-card {
  background: #fff;
  border-radius: 12px;
  padding: 20px;
  display: flex;
  align-items: center;
  gap: 16px;
  box-shadow: 0 2px 12px rgba(0, 0, 0, 0.04);
  transition: transform 0.3s, box-shadow 0.3s;
}

.stat-card:hover {
  transform: translateY(-2px);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.08);
}

.stat-icon {
  width: 48px;
  height: 48px;
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  font-size: 24px;
}

.stat-info {
  display: flex;
  flex-direction: column;
}

.stat-value {
  font-size: 24px;
  font-weight: 600;
  color: #303133;
}

.stat-label {
  font-size: 13px;
  color: #909399;
  margin-top: 4px;
}

/* 卡片通用样式 */
.strategy-card,
.status-card,
.health-card,
.record-types-card {
  border-radius: 12px;
  border: none;
  margin-bottom: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
}

.strategy-card :deep(.el-card__body),
.status-card :deep(.el-card__body),
.health-card :deep(.el-card__body),
.record-types-card :deep(.el-card__body) {
  flex: 1;
  display: flex;
  flex-direction: column;
}

/* 策略卡片内容布局 */
.strategy-card :deep(.el-card__body) > div {
  flex: 1;
  display: flex;
  flex-direction: column;
}

.strategy-options {
  flex: 1;
  margin-bottom: 20px;
}

/* 记录类型卡片内容布局 */
.record-types-card :deep(.el-card__body) > div {
  flex: 1;
  display: flex;
  flex-direction: column;
}

.record-type-list {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* 状态卡片内容布局 */
.status-content {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 16px;
}

@media (max-width: 768px) {
  .status-content {
    grid-template-columns: 1fr;
  }
}

/* 健康检查卡片内容布局 */
.health-card :deep(.el-card__body) > div {
  flex: 1;
  display: flex;
  flex-direction: column;
}

.health-list {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 16px;
}

@media (max-width: 1200px) {
  .health-list {
    grid-template-columns: repeat(2, 1fr);
  }
}

@media (max-width: 768px) {
  .health-list {
    grid-template-columns: 1fr;
  }
}

.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.card-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 16px;
  font-weight: 600;
  color: #303133;
}

.card-title .el-icon {
  color: #409eff;
}

/* 策略卡片 */
.current-strategy {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 8px;
}

.current-label {
  font-size: 14px;
  color: #909399;
}

.strategy-desc {
  font-size: 13px;
  color: #606266;
  margin: 0 0 16px 0;
  padding: 12px;
  background: #f5f7fa;
  border-radius: 8px;
}

.strategy-radio-group {
  height: 100%;
}

.strategy-item {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 10px 16px;
  background: #f5f7fa;
  border-radius: 8px;
  margin-bottom: 6px;
  cursor: pointer;
  transition: all 0.3s;
  border: 2px solid transparent;
  width: 100%;
  box-sizing: border-box;
}

.strategy-item:hover {
  background: #f0f2f5;
}

.strategy-item.is-selected {
  background: #ecf5ff;
  border-color: #409eff;
}

.strategy-content {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  text-align: left;
}

.strategy-name {
  font-size: 14px;
  font-weight: 600;
  color: #303133;
}

.strategy-option-desc {
  font-size: 12px;
  color: #909399;
  line-height: 1.5;
}

.save-btn {
  width: 100%;
  margin-top: auto;
}

/* 状态卡片 */
.status-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 12px 16px;
  background: #f5f7fa;
  border-radius: 8px;
}

.status-label {
  font-size: 14px;
  color: #606266;
}

.status-value {
  font-size: 14px;
  font-weight: 500;
  color: #303133;
}

.status-value.highlight {
  color: #409eff;
  font-size: 16px;
}

/* 记录类型开关 */
.section-desc {
  font-size: 13px;
  color: #909399;
  margin: 0 0 16px 0;
}

.record-type-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 12px 16px;
  background: #f5f7fa;
  border-radius: 8px;
  transition: all 0.3s;
}

.record-type-item:hover {
  background: #f0f2f5;
}

.record-type-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.record-type-name {
  font-size: 14px;
  font-weight: 600;
  color: #303133;
  font-family: 'Monaco', 'Menlo', monospace;
}

.record-type-desc {
  font-size: 12px;
  color: #909399;
}

/* 健康检查卡片 */
.health-card {
  margin-top: 0;
}

.health-item {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 16px;
  background: #f5f7fa;
  border-radius: 12px;
  transition: all 0.3s;
  border: 2px solid transparent;
}

.health-item.is-healthy {
  border-color: #67c23a;
  background: #f0f9eb;
}

.health-icon {
  width: 48px;
  height: 48px;
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  font-size: 24px;
  flex-shrink: 0;
}

.health-info {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.health-label {
  font-size: 15px;
  font-weight: 600;
  color: #303133;
}

.health-status {
  font-size: 13px;
  color: #f56c6c;
}

.health-status.is-ok {
  color: #67c23a;
}

.health-check-icon {
  font-size: 24px;
  color: #f56c6c;
}

.health-check-icon.is-ok {
  color: #67c23a;
}

/* 响应式 */
@media (max-width: 768px) {
  .page-header {
    flex-direction: column;
    gap: 16px;
  }

  .stat-card {
    padding: 16px;
  }

  .stat-value {
    font-size: 20px;
  }

  .health-item {
    margin-bottom: 12px;
  }

  .equal-height-row {
    margin-bottom: 20px;
  }

  .equal-height-row .el-col {
    margin-bottom: 20px;
  }

  .strategy-card :deep(.el-card__body),
  .record-types-card :deep(.el-card__body),
  .status-card :deep(.el-card__body),
  .health-card :deep(.el-card__body) {
    min-height: auto;
  }
}

/* 等高行 */
.equal-height-row {
  display: flex;
  flex-wrap: wrap;
  margin-bottom: 20px;
}

.equal-height-row > .el-col {
  display: flex;
}

.equal-height-row > .el-col > .el-card {
  flex: 1;
}

/* 日志清理卡片 */
.log-cleanup-card {
  border-radius: 12px;
  border: none;
}

.cleanup-section {
  padding: 16px;
  background: #f5f7fa;
  border-radius: 12px;
  height: 100%;
}

.section-title {
  margin: 0 0 8px 0;
  font-size: 15px;
  font-weight: 600;
  color: #303133;
}

.cleanup-form {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}

.oldest-log-info {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: #909399;
  padding: 8px 12px;
  background: #fff;
  border-radius: 6px;
}

.oldest-log-info .el-icon {
  color: #409eff;
}

.auto-cleanup-form {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.auto-cleanup-toggle {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  background: #fff;
  border-radius: 8px;
}

.toggle-label {
  font-size: 14px;
  color: #303133;
}

.retention-days-input {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  background: #fff;
  border-radius: 8px;
}

.input-label {
  font-size: 14px;
  color: #303133;
  min-width: 70px;
}

.input-suffix {
  font-size: 14px;
  color: #909399;
}
/* 响应式样式已针对大屏幕优化，以下为移动端适配 */
@media (max-width: 768px) {
  .page-header {
    flex-direction: column;
    align-items: stretch;
    gap: 16px;
  }

  .header-left h1 {
    font-size: 20px;
  }

  .stat-card {
    padding: 12px;
    gap: 10px;
  }

  .stat-icon {
    width: 36px;
    height: 36px;
    font-size: 18px;
    border-radius: 8px;
  }

  .stat-value {
    font-size: 18px;
  }

  .stat-label {
    font-size: 12px;
  }

  /* 清理表单适配 */
  .cleanup-form {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
  }

  .date-picker-responsive {
    width: 100% !important;
  }

  .cleanup-btns {
    display: flex;
    gap: 8px;
  }

  .cleanup-btns .el-button {
    flex: 1;
    margin-left: 0;
  }

  /* 自动清理适配 */
  .auto-cleanup-form {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
  }

  .number-input-responsive {
    width: 100% !important;
  }

  /* 记录类型适配 */
  .record-type-item {
    padding: 10px 0;
  }

  .record-type-desc {
    display: none;
  }

  /* 系统状态适配 */
  .status-item {
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    padding: 10px 0;
  }

  .status-value {
    font-size: 14px;
  }

  /* 健康检查适配 */
  .health-item {
    padding: 10px;
  }
}
</style>

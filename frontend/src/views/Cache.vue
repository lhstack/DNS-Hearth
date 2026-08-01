<template>
  <div class="cache-management">
    <!-- 页面标题 -->
    <div class="page-header">
      <div class="header-left">
        <h1>缓存管理</h1>
        <p class="subtitle">管理 DNS 缓存，查看统计信息，配置缓存策略</p>
      </div>
      <el-button type="primary" size="large" @click="fetchStats">
        <el-icon><Refresh /></el-icon>
        刷新统计
      </el-button>
    </div>

    <!-- 统计卡片 -->
    <el-row :gutter="20" class="stats-row">
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);">
            <el-icon><Box /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ stats.entries }}</span>
            <span class="stat-label">缓存条目</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: linear-gradient(135deg, #11998e 0%, #38ef7d 100%);">
            <el-icon><CircleCheck /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ stats.hits }}</span>
            <span class="stat-label">命中次数</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: linear-gradient(135deg, #f5576c 0%, #f093fb 100%);">
            <el-icon><CircleClose /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ stats.misses }}</span>
            <span class="stat-label">未命中次数</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: linear-gradient(135deg, #4facfe 0%, #00f2fe 100%);">
            <el-icon><TrendCharts /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ formatHitRate(stats.hit_rate * 100) }}</span>
            <span class="stat-label">命中率</span>
          </div>
        </div>
      </el-col>
    </el-row>

    <el-row :gutter="20" class="config-row">
      <!-- 缓存配置 -->
      <el-col :xs="24" :md="12">
        <el-card class="config-card" shadow="never">
          <template #header>
            <div class="card-title">
              <el-icon><Setting /></el-icon>
              <span>缓存配置</span>
            </div>
          </template>
          <el-form
            ref="configFormRef"
            :model="configForm"
            label-position="top"
            v-loading="loadingConfig"
          >
            <el-form-item label="默认 TTL（秒）">
              <el-input-number
                v-model="configForm.default_ttl"
                :min="1"
                :max="604800"
                :step="60"
                size="large"
                style="width: 100%"
              />
              <div class="form-tip">缓存条目的默认生存时间，范围 1-604800 秒</div>
            </el-form-item>
            <el-form-item label="最大条目数">
              <el-input-number
                v-model="configForm.max_entries"
                :min="1"
                :max="1000000"
                :step="1000"
                size="large"
                style="width: 100%"
              />
              <div class="form-tip">缓存可存储的最大条目数量</div>
            </el-form-item>
            <el-form-item>
              <el-button type="primary" @click="saveConfig" :loading="savingConfig" size="large">
                <el-icon><Check /></el-icon>
                保存配置
              </el-button>
            </el-form-item>
          </el-form>
        </el-card>
      </el-col>

      <!-- 命中率图表 -->
      <el-col :xs="24" :md="12">
        <el-card class="chart-card" shadow="never">
          <template #header>
            <div class="card-title">
              <el-icon><PieChart /></el-icon>
              <span>缓存命中分布</span>
            </div>
          </template>
          <div class="hit-rate-chart">
            <div class="chart-ring">
              <el-progress
                type="circle"
                :percentage="Number.isFinite(stats.hit_rate) ? stats.hit_rate * 100 : 0"
                :width="180"
                :stroke-width="16"
                :color="getHitRateColor(stats.hit_rate)"
              >
                <template #default>
                  <div class="chart-center">
                    <span class="chart-value">{{ formatHitRate(stats.hit_rate * 100) }}</span>
                    <span class="chart-label">命中率</span>
                  </div>
                </template>
              </el-progress>
            </div>
            <div class="chart-legend">
              <div class="legend-item">
                <span class="legend-dot" style="background: #67c23a;"></span>
                <span class="legend-label">命中</span>
                <span class="legend-value">{{ stats.hits }}</span>
              </div>
              <div class="legend-item">
                <span class="legend-dot" style="background: #f56c6c;"></span>
                <span class="legend-label">未命中</span>
                <span class="legend-value">{{ stats.misses }}</span>
              </div>
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 缓存操作 -->
    <el-card class="operations-card" shadow="never">
      <template #header>
        <div class="card-title">
          <el-icon><Operation /></el-icon>
          <span>缓存操作</span>
        </div>
      </template>
      <el-row :gutter="24">
        <el-col :xs="24" :md="8">
          <div class="operation-item">
            <div class="operation-icon" style="background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);">
              <el-icon><Search /></el-icon>
            </div>
            <div class="operation-content">
              <h4>清除指定域名缓存</h4>
              <p>清除特定域名的所有缓存记录</p>
              <el-input
                v-model="clearDomain"
                placeholder="输入域名，如 example.com"
                size="large"
                class="operation-input"
              >
                <template #append>
                  <el-button
                    type="primary"
                    @click="clearDomainCache"
                    :loading="clearingDomain"
                    :disabled="!clearDomain"
                  >
                    清除
                  </el-button>
                </template>
              </el-input>
            </div>
          </div>
        </el-col>
        <el-col :xs="24" :md="8">
          <div class="operation-item">
            <div class="operation-icon" style="background: linear-gradient(135deg, #f093fb 0%, #f5576c 100%);">
              <el-icon><Delete /></el-icon>
            </div>
            <div class="operation-content">
              <h4>清除全部缓存</h4>
              <p>清除所有缓存条目，此操作不可撤销</p>
              <el-button type="danger" size="large" @click="confirmClearAll" :loading="clearingAll">
                <el-icon><Delete /></el-icon>
                清除全部缓存
              </el-button>
            </div>
          </div>
        </el-col>
        <el-col :xs="24" :md="8">
          <div class="operation-item">
            <div class="operation-icon" style="background: linear-gradient(135deg, #fa709a 0%, #fee140 100%);">
              <el-icon><Brush /></el-icon>
            </div>
            <div class="operation-content">
              <h4>清理过期缓存</h4>
              <p>清理所有已过期的缓存条目，释放内存</p>
              <el-button type="warning" size="large" @click="cleanupExpired" :loading="cleaningUp">
                <el-icon><Brush /></el-icon>
                清理过期缓存
              </el-button>
            </div>
          </div>
        </el-col>
      </el-row>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  Refresh, Box, CircleCheck, CircleClose, TrendCharts,
  Setting, Check, PieChart, Operation, Search, Delete, Brush
} from '@element-plus/icons-vue'
import api from '../api'

interface CacheStats {
  hits: number
  misses: number
  entries: number
  hit_rate: number
}

interface CacheConfig {
  default_ttl: number
  max_entries: number
}

const stats = ref<CacheStats>({
  hits: 0,
  misses: 0,
  entries: 0,
  hit_rate: 0
})

const configForm = reactive<CacheConfig>({
  default_ttl: 60,
  max_entries: 10000
})

const loadingStats = ref(false)
const loadingConfig = ref(false)
const savingConfig = ref(false)
const clearDomain = ref('')
const clearingDomain = ref(false)
const clearingAll = ref(false)
const cleaningUp = ref(false)

function formatHitRate(percentage: number): string {
  if (!Number.isFinite(percentage)) return '数据异常'
  return `${percentage.toFixed(1)}%`
}

function getHitRateColor(rate: number): string {
  if (rate >= 0.8) return '#67c23a'
  if (rate >= 0.5) return '#e6a23c'
  return '#f56c6c'
}

async function fetchStats() {
  loadingStats.value = true
  try {
    const response = await api.get('/api/cache/stats')
    const value = response.data as CacheStats
    if (!Number.isFinite(value.hit_rate)) {
      throw new Error('缓存统计返回了无效的命中率')
    }
    stats.value = value
  } catch (error: any) {
    ElMessage.error(error?.message || error?.response?.data?.message || '获取缓存统计失败')
  } finally {
    loadingStats.value = false
  }
}

async function fetchConfig() {
  loadingConfig.value = true
  try {
    const response = await api.get('/api/cache/config')
    configForm.default_ttl = response.data.default_ttl
    configForm.max_entries = response.data.max_entries
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '获取缓存配置失败')
  } finally {
    loadingConfig.value = false
  }
}

async function saveConfig() {
  savingConfig.value = true
  try {
    await api.put('/api/cache/config', configForm)
    ElMessage.success('缓存配置已保存')
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '保存配置失败')
  } finally {
    savingConfig.value = false
  }
}

async function clearDomainCache() {
  if (!clearDomain.value) return

  clearingDomain.value = true
  try {
    await api.post(`/api/cache/clear/${encodeURIComponent(clearDomain.value)}`)
    ElMessage.success(`已清除域名 ${clearDomain.value} 的缓存`)
    clearDomain.value = ''
    fetchStats()
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '清除缓存失败')
  } finally {
    clearingDomain.value = false
  }
}

async function confirmClearAll() {
  try {
    await ElMessageBox.confirm(
      '确定要清除全部缓存吗？此操作不可撤销。',
      '确认清除',
      {
        confirmButtonText: '清除',
        cancelButtonText: '取消',
        type: 'warning'
      }
    )
    clearingAll.value = true
    await api.post('/api/cache/clear')
    ElMessage.success('全部缓存已清除')
    fetchStats()
  } catch (error: any) {
    if (error !== 'cancel') {
      ElMessage.error(error.response?.data?.message || '清除缓存失败')
    }
  } finally {
    clearingAll.value = false
  }
}

async function cleanupExpired() {
  cleaningUp.value = true
  try {
    const response = await api.post('/api/cache/cleanup')
    ElMessage.success(`过期缓存已清理，剩余 ${response.data.remaining_entries} 条`)
    fetchStats()
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '清理缓存失败')
  } finally {
    cleaningUp.value = false
  }
}

onMounted(() => {
  fetchStats()
  fetchConfig()
})
</script>

<style scoped>
.cache-management {
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

/* 配置行 - 等高卡片 */
.config-row {
  display: flex;
  align-items: stretch;
}

.config-row > .el-col {
  display: flex;
}

/* 卡片样式 */
.config-card,
.chart-card,
.operations-card {
  border-radius: 12px;
  border: none;
  margin-bottom: 20px;
  width: 100%;
}

.config-card,
.chart-card {
  display: flex;
  flex-direction: column;
}

.config-card :deep(.el-card__body),
.chart-card :deep(.el-card__body) {
  flex: 1;
  display: flex;
  flex-direction: column;
}

.config-card :deep(.el-form) {
  flex: 1;
  display: flex;
  flex-direction: column;
}

.config-card :deep(.el-form-item:last-child) {
  margin-top: auto;
  margin-bottom: 0;
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
  color: #667eea;
}

.form-tip {
  font-size: 12px;
  color: #909399;
  margin-top: 4px;
}

/* 命中率图表 */
.hit-rate-chart {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 20px 0;
  flex: 1;
}

.chart-ring {
  margin-bottom: 24px;
}

.chart-center {
  display: flex;
  flex-direction: column;
  align-items: center;
}

.chart-value {
  font-size: 28px;
  font-weight: 600;
  color: #303133;
}

.chart-label {
  font-size: 14px;
  color: #909399;
}

.chart-legend {
  display: flex;
  gap: 32px;
}

.legend-item {
  display: flex;
  align-items: center;
  gap: 8px;
}

.legend-dot {
  width: 12px;
  height: 12px;
  border-radius: 50%;
}

.legend-label {
  font-size: 14px;
  color: #606266;
}

.legend-value {
  font-size: 14px;
  font-weight: 600;
  color: #303133;
}

/* 操作区域 */
.operation-item {
  display: flex;
  gap: 16px;
  padding: 20px;
  background: #f8f9fa;
  border-radius: 12px;
  height: 100%;
}

.operation-icon {
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

.operation-content {
  flex: 1;
}

.operation-content h4 {
  margin: 0 0 8px 0;
  font-size: 15px;
  font-weight: 600;
  color: #303133;
}

.operation-content p {
  margin: 0 0 16px 0;
  font-size: 13px;
  color: #909399;
}

.operation-input {
  max-width: 100%;
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

  .operation-item {
    flex-direction: column;
    text-align: center;
  }

  .operation-icon {
    margin: 0 auto;
  }
}
</style>

<template>
  <div class="upstreams">
    <!-- 页面标题 -->
    <div class="page-header">
      <div class="header-left">
        <h1>上游服务器管理</h1>
        <p class="subtitle">配置 DNS 上游服务器，支持 UDP、DoT、DoH、DoQ、DoH3 协议</p>
      </div>
      <el-button type="primary" size="large" @click="openCreateDialog">
        <el-icon><Plus /></el-icon>
        添加服务器
      </el-button>
    </div>

    <!-- 统计卡片 -->
    <el-row :gutter="20" class="stats-row">
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: #409eff;">
            <el-icon><Connection /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ pagination.total }}</span>
            <span class="stat-label">服务器</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: #67c23a;">
            <el-icon><CircleCheck /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ healthyCount }}</span>
            <span class="stat-label">健康</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: #f56c6c;">
            <el-icon><Warning /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ unhealthyCount }}</span>
            <span class="stat-label">异常</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: #e6a23c;">
            <el-icon><DataAnalysis /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ totalQueries }}</span>
            <span class="stat-label">总查询</span>
          </div>
        </div>
      </el-col>
    </el-row>

    <!-- 服务器表格 -->
    <el-card class="table-card" shadow="never">
      <div class="table-wrapper">
        <el-table :data="servers" v-loading="loading" stripe class="custom-table">
          <el-table-column prop="id" label="ID" width="80" class-name="hidden-xs-only" />
          <el-table-column prop="name" label="名称" min-width="120" show-overflow-tooltip>
            <template #default="{ row }">
              <span class="server-name">{{ row.name }}</span>
            </template>
          </el-table-column>
          <el-table-column prop="address" label="地址" min-width="220" show-overflow-tooltip>
            <template #default="{ row }">
              <span class="server-address">{{ row.address }}</span>
            </template>
          </el-table-column>
          <el-table-column prop="protocol" label="协议" width="90">
            <template #default="{ row }">
              <el-tag :type="getProtocolTag(row.protocol)" effect="dark" size="small">
                {{ row.protocol.toUpperCase() }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column prop="timeout" label="超时" width="80" class-name="hidden-xs-only">
            <template #default="{ row }">
              <span class="timeout-value">{{ row.timeout }}ms</span>
            </template>
          </el-table-column>
          <el-table-column label="状态" width="80">
            <template #default="{ row }">
              <el-tag :type="getStatusTag(row)" effect="plain" size="small">
                {{ getStatusLabel(row) }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column label="统计" width="220">
            <template #default="{ row }">
              <div class="stats-cell">
                <div class="stats-item">
                  <span class="stats-label">查询</span>
                  <span class="stats-value">{{ getServerStats(row.id)?.queries || 0 }}</span>
                </div>
                <div class="stats-item">
                  <span class="stats-label">成功率</span>
                  <span class="stats-value" :class="getSuccessRateClass(getServerStats(row.id)?.success_rate)">
                    {{ formatSuccessRate(getServerStats(row.id)?.success_rate) }}
                  </span>
                </div>
                <div class="stats-item">
                  <span class="stats-label">延迟</span>
                  <span class="stats-value">{{ formatResponseTime(getServerStats(row.id)?.avg_response_time_ms) }}</span>
                </div>
              </div>
            </template>
          </el-table-column>
          <el-table-column prop="enabled" label="启用" width="70">
            <template #default="{ row }">
              <el-switch
                v-model="row.enabled"
                @change="toggleEnabled(row)"
                inline-prompt
                active-text="启"
                inactive-text="停"
                size="small"
              />
            </template>
          </el-table-column>
          <el-table-column label="操作" width="140" fixed="right">
            <template #default="{ row }">
              <el-button
                type="primary"
                link
                :icon="Edit"
                class="row-action"
                aria-label="编辑服务器"
                @click="openEditDialog(row)"
              />
              <el-button
                v-if="canResetHealth(row)"
                type="warning"
                link
                :icon="RefreshRight"
                class="row-action"
                aria-label="重置健康状态"
                @click="resetHealth(row)"
                :loading="resettingHealth === row.id"
              />
              <el-button
                type="danger"
                link
                :icon="Delete"
                class="row-action"
                aria-label="删除服务器"
                @click="confirmDelete(row)"
              />
            </template>
          </el-table-column>
          <template #empty>
            <el-empty description="暂无上游服务器" />
          </template>
        </el-table>
      </div>

      <div class="pagination-container">
        <el-pagination
          v-model:current-page="pagination.page"
          v-model:page-size="pagination.pageSize"
          :page-sizes="[10, 20, 50, 100]"
          :total="pagination.total"
          layout="total, sizes, prev, pager, next"
          @size-change="handleSizeChange"
          @current-change="handlePageChange"
        />
      </div>
    </el-card>

    <!-- 创建/编辑对话框 -->
    <el-dialog
      v-model="dialogVisible"
      :title="isEditing ? '编辑服务器' : '添加服务器'"
      :width="isMobile ? '90%' : '520px'"
      class="custom-dialog"
    >
      <el-form
        ref="formRef"
        :model="formData"
        :rules="formRules"
        label-position="top"
      >
        <el-form-item label="名称" prop="name">
          <el-input v-model="formData.name" placeholder="Cloudflare DNS" size="large" />
        </el-form-item>
        <el-row :gutter="16">
          <el-col :xs="24" :sm="12">
            <el-form-item label="协议" prop="protocol">
              <el-select v-model="formData.protocol" placeholder="选择协议" size="large" style="width: 100%">
                <el-option label="UDP" value="udp" />
                <el-option label="DoT (DNS over TLS)" value="dot" />
                <el-option label="DoH (DNS over HTTPS)" value="doh" />
                <el-option label="DoQ (DNS over QUIC)" value="doq" />
                <el-option label="DoH3 (DNS over HTTP/3)" value="doh3" />
              </el-select>
            </el-form-item>
          </el-col>
          <el-col :xs="24" :sm="12">
            <el-form-item label="超时 (毫秒)" prop="timeout">
              <el-input-number
                v-model="formData.timeout"
                :min="100"
                :max="60000"
                :step="100"
                size="large"
                style="width: 100%"
              />
            </el-form-item>
          </el-col>
        </el-row>
        <el-form-item label="地址" prop="address">
          <el-input
            v-model="formData.address"
            :placeholder="getAddressPlaceholder(formData.protocol)"
            size="large"
          />
          <div class="form-tip">{{ getAddressTip(formData.protocol) }}</div>
        </el-form-item>
        <el-form-item label="状态" prop="enabled">
          <el-switch v-model="formData.enabled" active-text="启用" inactive-text="禁用" size="large" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialogVisible = false" size="large">取消</el-button>
        <el-button type="primary" @click="submitForm" :loading="submitting" size="large">
          {{ isEditing ? '保存修改' : '创建服务器' }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted, onUnmounted } from 'vue'
import { ElMessage, ElMessageBox, type FormInstance, type FormRules } from 'element-plus'
import { Plus, Edit, Delete, Connection, CircleCheck, Warning, DataAnalysis, RefreshRight } from '@element-plus/icons-vue'
import api from '../api'
import { useResponsive } from '../composables/useResponsive'

const { isMobile } = useResponsive()

interface UpstreamServer {
  id: number
  name: string
  address: string
  protocol: string
  timeout: number
  enabled: boolean
  created_at: string
  updated_at: string
}

interface ServerStatus {
  id: number
  name: string
  address: string
  protocol: string
  enabled: boolean
  healthy: boolean
  queries: number
  successes: number
  failures: number
  success_rate: number
  avg_response_time_ms: number
  suspended: boolean
  suspension_remaining_secs: number | null
}

const servers = ref<UpstreamServer[]>([])
const serverStatus = ref<Map<number, ServerStatus>>(new Map())
const loading = ref(false)
const dialogVisible = ref(false)
const isEditing = ref(false)
const submitting = ref(false)
const formRef = ref<FormInstance>()
const editingId = ref<number | null>(null)
const resettingHealth = ref<number | null>(null)
let statusInterval: ReturnType<typeof setInterval> | null = null

const pagination = reactive({
  page: 1,
  pageSize: 20,
  total: 0
})

const healthyCount = computed(() => {
  let count = 0
  serverStatus.value.forEach(s => { if (s.healthy && s.enabled) count++ })
  return count
})

const unhealthyCount = computed(() => {
  let count = 0
  serverStatus.value.forEach(s => { if (!s.healthy && s.enabled) count++ })
  return count
})

const totalQueries = computed(() => {
  let total = 0
  serverStatus.value.forEach(s => { total += s.queries })
  return total
})

const formData = reactive({
  name: '',
  address: '',
  protocol: 'udp',
  timeout: 5000,
  enabled: true
})

const formRules: FormRules = {
  name: [
    { required: true, message: '请输入服务器名称', trigger: 'blur' },
    { max: 100, message: '名称长度不能超过100个字符', trigger: 'blur' }
  ],
  protocol: [
    { required: true, message: '请选择协议', trigger: 'change' }
  ],
  address: [
    { required: true, message: '请输入服务器地址', trigger: 'blur' }
  ],
  timeout: [
    { required: true, message: '请输入超时时间', trigger: 'blur' }
  ]
}

function getProtocolTag(protocol: string): string {
  const tags: Record<string, string> = {
    udp: 'info',
    dot: 'success',
    doh: 'warning',
    doq: '',
    doh3: 'danger'
  }
  return tags[protocol] || ''
}

function getStatusTag(server: UpstreamServer): string {
  if (!server.enabled) return 'info'
  const status = serverStatus.value.get(server.id)
  if (!status) return 'info'
  if (status.suspended) return 'warning'
  return status.healthy ? 'success' : 'danger'
}

function getStatusLabel(server: UpstreamServer): string {
  if (!server.enabled) return '已禁用'
  const status = serverStatus.value.get(server.id)
  if (!status) return '未知'
  if (status.suspended) {
    const remaining = status.suspension_remaining_secs
    if (remaining && remaining > 0) {
      return `暂停${remaining}s`
    }
    return '暂停中'
  }
  return status.healthy ? '健康' : '异常'
}

function getServerStats(id: number): ServerStatus | undefined {
  return serverStatus.value.get(id)
}

function formatResponseTime(ms: number | undefined): string {
  if (ms === undefined || ms === null) return '-'
  // 如果是 u64::MAX 或非常大的数字，说明没有数据
  if (ms > 100000) return '-'
  return `${Math.round(ms)}ms`
}

function formatSuccessRate(rate: number | undefined): string {
  if (rate === undefined || rate === null) return '-'
  return `${(rate * 100).toFixed(1)}%`
}

function getSuccessRateClass(rate: number | undefined): string {
  if (rate === undefined || rate === null) return ''
  if (rate >= 0.95) return 'success-rate-good'
  if (rate >= 0.8) return 'success-rate-warning'
  return 'success-rate-danger'
}

function canResetHealth(server: UpstreamServer): boolean {
  if (!server.enabled) return false
  const status = serverStatus.value.get(server.id)
  if (!status) return false
  return status.suspended || !status.healthy
}

async function resetHealth(server: UpstreamServer) {
  resettingHealth.value = server.id
  try {
    await api.post(`/api/upstreams/${server.id}/reset-health`)
    ElMessage.success(`服务器 "${server.name}" 健康状态已重置`)
    fetchStatus()
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '重置健康状态失败')
  } finally {
    resettingHealth.value = null
  }
}

function getAddressPlaceholder(protocol: string): string {
  const placeholders: Record<string, string> = {
    udp: '8.8.8.8:53',
    dot: '1.1.1.1:853',
    doh: 'https://dns.google/dns-query',
    doq: 'dns.adguard-dns.com:853',
    doh3: 'https://dns.adguard-dns.com/dns-query'
  }
  return placeholders[protocol] || ''
}

function getAddressTip(protocol: string): string {
  const tips: Record<string, string> = {
    udp: '格式: IP:端口，如 8.8.8.8:53',
    dot: '格式: 域名:端口，如 dns.google:853',
    doh: '格式: HTTPS URL，如 https://dns.google/dns-query',
    doq: '格式: 域名:端口，如 dns.adguard-dns.com:853',
    doh3: '格式: HTTPS URL，如 https://dns.adguard-dns.com/dns-query'
  }
  return tips[protocol] || ''
}

async function fetchServers() {
  loading.value = true
  try {
    const response = await api.get('/api/upstreams', {
      params: {
        page: pagination.page,
        page_size: pagination.pageSize
      }
    })
    servers.value = response.data.data
    pagination.total = response.data.total
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '获取服务器列表失败')
  } finally {
    loading.value = false
  }
}

function handleSizeChange(size: number) {
  pagination.pageSize = size
  pagination.page = 1
  fetchServers()
}

function handlePageChange(page: number) {
  pagination.page = page
  fetchServers()
}

async function fetchStatus() {
  try {
    const response = await api.get('/api/upstreams/status')
    const statusMap = new Map<number, ServerStatus>()
    for (const status of response.data.data) {
      statusMap.set(status.id, status)
    }
    serverStatus.value = statusMap
  } catch {
    // Silently fail
  }
}

function resetForm() {
  formData.name = ''
  formData.address = ''
  formData.protocol = 'udp'
  formData.timeout = 5000
  formData.enabled = true
  editingId.value = null
}

function openCreateDialog() {
  isEditing.value = false
  resetForm()
  dialogVisible.value = true
}

function openEditDialog(server: UpstreamServer) {
  isEditing.value = true
  editingId.value = server.id
  formData.name = server.name
  formData.address = server.address
  formData.protocol = server.protocol
  formData.timeout = server.timeout
  formData.enabled = server.enabled
  dialogVisible.value = true
}

async function submitForm() {
  if (!formRef.value) return

  await formRef.value.validate(async (valid) => {
    if (!valid) return

    submitting.value = true
    try {
      if (isEditing.value && editingId.value) {
        await api.put(`/api/upstreams/${editingId.value}`, formData)
        ElMessage.success('服务器更新成功')
      } else {
        await api.post('/api/upstreams', formData)
        ElMessage.success('服务器创建成功')
      }
      dialogVisible.value = false
      fetchServers()
      fetchStatus()
    } catch (error: any) {
      const message = error.response?.data?.message || '操作失败'
      ElMessage.error(message)
    } finally {
      submitting.value = false
    }
  })
}

async function toggleEnabled(server: UpstreamServer) {
  try {
    await api.put(`/api/upstreams/${server.id}`, { enabled: server.enabled })
    ElMessage.success(server.enabled ? '服务器已启用' : '服务器已禁用')
    fetchStatus()
  } catch (error: any) {
    server.enabled = !server.enabled
    ElMessage.error(error.response?.data?.message || '操作失败')
  }
}

async function confirmDelete(server: UpstreamServer) {
  try {
    await ElMessageBox.confirm(
      `确定要删除服务器 "${server.name}" 吗？`,
      '确认删除',
      {
        confirmButtonText: '删除',
        cancelButtonText: '取消',
        type: 'warning'
      }
    )
    await api.delete(`/api/upstreams/${server.id}`)
    ElMessage.success('服务器删除成功')
    fetchServers()
  } catch (error: any) {
    if (error !== 'cancel') {
      ElMessage.error(error.response?.data?.message || '删除失败')
    }
  }
}

onMounted(() => {
  fetchServers()
  fetchStatus()
  statusInterval = setInterval(fetchStatus, 30000)
})

onUnmounted(() => {
  if (statusInterval) clearInterval(statusInterval)
})
</script>

<style scoped>
.upstreams {
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

/* 表格卡片 */
.table-card {
  border-radius: 12px;
  border: none;
}

.table-card :deep(.el-card__body) {
  padding: 0;
}

.custom-table :deep(.el-table__header th) {
  background: #f5f7fa;
  color: #606266;
  font-weight: 600;
}

.server-name {
  font-weight: 500;
  color: #303133;
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.server-address {
  font-family: 'Monaco', 'Menlo', monospace;
  font-size: 13px;
  color: #606266;
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.timeout-value {
  color: #909399;
}

.stats-cell {
  display: flex;
  gap: 16px;
}

.stats-item {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.stats-label {
  font-size: 11px;
  color: #909399;
}

.stats-value {
  font-size: 13px;
  font-weight: 500;
  color: #303133;
}

.stats-value.success-rate-good {
  color: #67c23a;
}

.stats-value.success-rate-warning {
  color: #e6a23c;
}

.stats-value.success-rate-danger {
  color: #f56c6c;
}

.pagination-container {
  display: flex;
  justify-content: flex-end;
  padding: 16px 20px;
  border-top: 1px solid #f0f0f0;
}


/* 对话框 */
.custom-dialog :deep(.el-dialog__header) {
  border-bottom: 1px solid #f0f0f0;
  padding: 20px 24px;
}

.custom-dialog :deep(.el-dialog__body) {
  padding: 24px;
}

.custom-dialog :deep(.el-dialog__footer) {
  border-top: 1px solid #f0f0f0;
  padding: 16px 24px;
}

.form-tip {
  font-size: 12px;
  color: #909399;
  margin-top: 4px;
}

/* 表格包装器 */
.table-wrapper {
  overflow-x: auto;
  -webkit-overflow-scrolling: touch;
}

/* 响应式 */
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

  .pagination-container {
    justify-content: center;
    padding: 12px;
  }

  .pagination-container :deep(.el-pagination) {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }

  .pagination-container :deep(.el-pagination__total),
  .pagination-container :deep(.el-pagination__sizes) {
    display: none;
  }
}
</style>

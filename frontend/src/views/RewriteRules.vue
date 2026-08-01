<template>
  <div class="rewrite-rules">
    <!-- 页面标题 -->
    <div class="page-header">
      <div class="header-left">
        <h1>重写规则管理</h1>
        <p class="subtitle">配置 DNS 查询重写规则</p>
      </div>
      <div class="header-actions">
        <el-button type="success" @click="openBatchDialog" class="action-btn">
          <el-icon><Upload /></el-icon>
          <span class="hidden-xs-only">批量导入</span>
        </el-button>
        <el-button type="primary" @click="openCreateDialog" class="action-btn">
          <el-icon><Plus /></el-icon>
          <span class="hidden-xs-only">添加规则</span>
        </el-button>
      </div>
    </div>

    <!-- 统计卡片 -->
    <el-row :gutter="20" class="stats-row">
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: #409eff;">
            <el-icon><Edit /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ rules.length }}</span>
            <span class="stat-label">总数</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: #67c23a;">
            <el-icon><CircleCheck /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ enabledCount }}</span>
            <span class="stat-label">启用</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: #f56c6c;">
            <el-icon><CloseBold /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ blockCount }}</span>
            <span class="stat-label">阻止</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: #e6a23c;">
            <el-icon><Switch /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ mapCount }}</span>
            <span class="stat-label">映射</span>
          </div>
        </div>
      </el-col>
    </el-row>

    <!-- 规则表格 -->
    <el-card class="table-card" shadow="never">
      <div class="table-wrapper">
        <el-table :data="rules" v-loading="loading" stripe class="custom-table">
          <el-table-column prop="id" label="ID" width="70" class-name="hidden-xs-only" />
          <el-table-column prop="pattern" label="匹配模式" min-width="180">
            <template #default="{ row }">
              <span class="pattern-text">{{ row.pattern }}</span>
            </template>
          </el-table-column>
          <el-table-column prop="match_type" label="类型" width="90">
            <template #default="{ row }">
              <el-tag :type="getMatchTypeTag(row.match_type)" effect="plain" size="small">
                {{ getMatchTypeLabel(row.match_type) }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column prop="action_type" label="动作" width="90">
            <template #default="{ row }">
              <el-tag :type="getActionTypeTag(row.action_type)" effect="dark" size="small">
                {{ getActionTypeLabel(row.action_type) }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column prop="action_value" label="动作值" min-width="150" class-name="hidden-xs-only">
            <template #default="{ row }">
              <span class="action-value">{{ row.action_value || '-' }}</span>
            </template>
          </el-table-column>
          <el-table-column prop="priority" label="优先级" width="80" class-name="hidden-xs-only" />
          <el-table-column prop="enabled" label="状态" width="80">
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
          <el-table-column label="操作" width="120" fixed="right">
            <template #default="{ row }">
              <el-button type="primary" link :icon="Edit" class="row-action" aria-label="编辑" @click="openEditDialog(row)" />
              <el-button type="danger" link :icon="Delete" class="row-action" aria-label="删除" @click="confirmDelete(row)" />
            </template>
          </el-table-column>
          <template #empty>
            <el-empty description="暂无重写规则" />
          </template>
        </el-table>
      </div>
    </el-card>

    <!-- 批量导入对话框 -->
    <el-dialog
      v-model="batchDialogVisible"
      title="批量导入规则"
      :width="isMobile ? '90%' : '600px'"
      class="custom-dialog"
    >
      <el-form
        ref="batchFormRef"
        :model="batchFormData"
        :rules="batchFormRules"
        label-position="top"
      >
        <el-form-item label="域名列表" prop="patterns">
          <el-input
            v-model="batchFormData.patterns"
            type="textarea"
            :rows="8"
            placeholder="每行一个域名，例如：&#10;ads.example.com&#10;tracker.example.com&#10;*.ads.com"
          />
          <div class="form-hint">支持换行、逗号、分号分隔，每个域名将创建一条规则</div>
        </el-form-item>
        <el-row :gutter="16">
          <el-col :xs="24" :sm="12">
            <el-form-item label="匹配类型" prop="match_type">
              <el-select v-model="batchFormData.match_type" placeholder="选择匹配类型" size="large" style="width: 100%">
                <el-option label="精确匹配" value="exact" />
                <el-option label="通配符" value="wildcard" />
                <el-option label="正则表达式" value="regex" />
              </el-select>
            </el-form-item>
          </el-col>
          <el-col :xs="24" :sm="12">
            <el-form-item label="动作类型" prop="action_type">
              <el-select v-model="batchFormData.action_type" placeholder="选择动作类型" size="large" style="width: 100%">
                <el-option label="阻止" value="block" />
                <el-option label="映射到 IP" value="map_ip" />
                <el-option label="映射到域名" value="map_domain" />
              </el-select>
            </el-form-item>
          </el-col>
        </el-row>
        <el-form-item
          v-if="batchFormData.action_type !== 'block'"
          label="动作值"
          prop="action_value"
        >
          <el-input
            v-model="batchFormData.action_value"
            :placeholder="getActionValuePlaceholder(batchFormData.action_type)"
            size="large"
          />
        </el-form-item>
        <el-row :gutter="16">
          <el-col :xs="24" :sm="12">
            <el-form-item label="优先级" prop="priority">
              <el-input-number v-model="batchFormData.priority" :min="0" size="large" style="width: 100%" />
            </el-form-item>
          </el-col>
          <el-col :xs="24" :sm="12">
            <el-form-item label="状态" prop="enabled">
              <div style="height: 40px; display: flex; align-items: center">
                <el-switch v-model="batchFormData.enabled" active-text="启用" inactive-text="禁用" size="large" />
              </div>
            </el-form-item>
          </el-col>
        </el-row>
        <el-form-item label="描述" prop="description">
          <el-input
            v-model="batchFormData.description"
            type="textarea"
            :rows="2"
            placeholder="批量规则描述（可选）"
            size="large"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="batchDialogVisible = false" size="large">取消</el-button>
        <el-button type="primary" @click="submitBatchForm" :loading="batchSubmitting" size="large">
          批量创建
        </el-button>
      </template>
    </el-dialog>

    <!-- 创建/编辑对话框 -->
    <el-dialog
      v-model="dialogVisible"
      :title="isEditing ? '编辑规则' : '添加规则'"
      :width="isMobile ? '90%' : '560px'"
      class="custom-dialog"
    >
      <el-form
        ref="formRef"
        :model="formData"
        :rules="formRules"
        label-position="top"
      >
        <el-form-item label="匹配模式" prop="pattern">
          <el-input
            v-model="formData.pattern"
            :placeholder="getPatternPlaceholder(formData.match_type)"
            size="large"
          />
        </el-form-item>
        <el-row :gutter="16">
          <el-col :xs="24" :sm="12">
            <el-form-item label="匹配类型" prop="match_type">
              <el-select v-model="formData.match_type" placeholder="选择匹配类型" size="large" style="width: 100%">
                <el-option label="精确匹配" value="exact" />
                <el-option label="通配符" value="wildcard" />
                <el-option label="正则表达式" value="regex" />
              </el-select>
            </el-form-item>
          </el-col>
          <el-col :xs="24" :sm="12">
            <el-form-item label="动作类型" prop="action_type">
              <el-select v-model="formData.action_type" placeholder="选择动作类型" size="large" style="width: 100%">
                <el-option label="映射到 IP" value="map_ip" />
                <el-option label="映射到域名" value="map_domain" />
                <el-option label="阻止" value="block" />
              </el-select>
            </el-form-item>
          </el-col>
        </el-row>
        <el-form-item
          v-if="formData.action_type !== 'block'"
          label="动作值"
          prop="action_value"
        >
          <el-input
            v-model="formData.action_value"
            :placeholder="getActionValuePlaceholder(formData.action_type)"
            size="large"
          />
        </el-form-item>
        <el-row :gutter="16">
          <el-col :xs="24" :sm="12">
            <el-form-item label="优先级" prop="priority">
              <el-input-number v-model="formData.priority" :min="0" size="large" style="width: 100%" />
            </el-form-item>
          </el-col>
          <el-col :xs="24" :sm="12">
            <el-form-item label="状态" prop="enabled">
              <div style="height: 40px; display: flex; align-items: center">
                <el-switch v-model="formData.enabled" active-text="启用" inactive-text="禁用" size="large" />
              </div>
            </el-form-item>
          </el-col>
        </el-row>
        <el-form-item label="描述" prop="description">
          <el-input
            v-model="formData.description"
            type="textarea"
            :rows="2"
            placeholder="规则描述（可选）"
            size="large"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialogVisible = false" size="large">取消</el-button>
        <el-button type="primary" @click="submitForm" :loading="submitting" size="large">
          {{ isEditing ? '保存修改' : '创建规则' }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox, type FormInstance, type FormRules } from 'element-plus'
import { Plus, Edit, Delete, CircleCheck, CloseBold, Switch, Upload } from '@element-plus/icons-vue'
import api from '../api'
import { useResponsive } from '../composables/useResponsive'

const { isMobile } = useResponsive()

interface RewriteRule {
  id: number
  pattern: string
  match_type: string
  action_type: string
  action_value: string | null
  priority: number
  enabled: boolean
  description: string | null
  created_at: string
  updated_at: string
}

const rules = ref<RewriteRule[]>([])
const loading = ref(false)
const dialogVisible = ref(false)
const batchDialogVisible = ref(false)
const isEditing = ref(false)
const submitting = ref(false)
const batchSubmitting = ref(false)
const formRef = ref<FormInstance>()
const batchFormRef = ref<FormInstance>()
const editingId = ref<number | null>(null)

const enabledCount = computed(() => rules.value.filter(r => r.enabled).length)
const blockCount = computed(() => rules.value.filter(r => r.action_type === 'block').length)
const mapCount = computed(() => rules.value.filter(r => r.action_type !== 'block').length)

const formData = reactive({
  pattern: '',
  match_type: 'exact',
  action_type: 'block',
  action_value: '',
  priority: 0,
  description: '',
  enabled: true
})

const formRules: FormRules = {
  pattern: [
    { required: true, message: '请输入匹配模式', trigger: 'blur' },
    { max: 255, message: '匹配模式长度不能超过255个字符', trigger: 'blur' }
  ],
  match_type: [
    { required: true, message: '请选择匹配类型', trigger: 'change' }
  ],
  action_type: [
    { required: true, message: '请选择动作类型', trigger: 'change' }
  ]
}

const batchFormData = reactive({
  patterns: '',
  match_type: 'exact',
  action_type: 'block',
  action_value: '',
  priority: 0,
  description: '',
  enabled: true
})

const batchFormRules: FormRules = {
  patterns: [
    { required: true, message: '请输入域名列表', trigger: 'blur' }
  ],
  match_type: [
    { required: true, message: '请选择匹配类型', trigger: 'change' }
  ],
  action_type: [
    { required: true, message: '请选择动作类型', trigger: 'change' }
  ]
}

function getMatchTypeLabel(type: string): string {
  const labels: Record<string, string> = {
    exact: '精确匹配',
    wildcard: '通配符',
    regex: '正则'
  }
  return labels[type] || type
}

function getMatchTypeTag(type: string): string {
  const tags: Record<string, string> = {
    exact: '',
    wildcard: 'warning',
    regex: 'danger'
  }
  return tags[type] || ''
}

function getActionTypeLabel(type: string): string {
  const labels: Record<string, string> = {
    map_ip: '映射 IP',
    map_domain: '映射域名',
    block: '阻止'
  }
  return labels[type] || type
}

function getActionTypeTag(type: string): string {
  const tags: Record<string, string> = {
    map_ip: 'success',
    map_domain: 'warning',
    block: 'danger'
  }
  return tags[type] || ''
}

function getPatternPlaceholder(matchType: string): string {
  const placeholders: Record<string, string> = {
    exact: 'ads.example.com',
    wildcard: '*.ads.com',
    regex: '^ads?\\.'
  }
  return placeholders[matchType] || ''
}

function getActionValuePlaceholder(actionType: string): string {
  const placeholders: Record<string, string> = {
    map_ip: '192.168.1.1 或 ::1',
    map_domain: 'target.example.com'
  }
  return placeholders[actionType] || ''
}

async function fetchRules() {
  loading.value = true
  try {
    const response = await api.get('/api/rewrite')
    rules.value = response.data.data
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '获取规则失败')
  } finally {
    loading.value = false
  }
}

function resetForm() {
  formData.pattern = ''
  formData.match_type = 'exact'
  formData.action_type = 'block'
  formData.action_value = ''
  formData.priority = 0
  formData.description = ''
  formData.enabled = true
  editingId.value = null
}

function openCreateDialog() {
  isEditing.value = false
  resetForm()
  dialogVisible.value = true
}

function openBatchDialog() {
  batchFormData.patterns = ''
  batchFormData.match_type = 'exact'
  batchFormData.action_type = 'block'
  batchFormData.action_value = ''
  batchFormData.priority = 0
  batchFormData.description = ''
  batchFormData.enabled = true
  batchDialogVisible.value = true
}

function openEditDialog(rule: RewriteRule) {
  isEditing.value = true
  editingId.value = rule.id
  formData.pattern = rule.pattern
  formData.match_type = rule.match_type
  formData.action_type = rule.action_type
  formData.action_value = rule.action_value || ''
  formData.priority = rule.priority
  formData.description = rule.description || ''
  formData.enabled = rule.enabled
  dialogVisible.value = true
}

async function submitForm() {
  if (!formRef.value) return

  await formRef.value.validate(async (valid) => {
    if (!valid) return

    submitting.value = true
    try {
      const payload = {
        ...formData,
        action_value: formData.action_type === 'block' ? null : formData.action_value || null,
        description: formData.description || null
      }

      if (isEditing.value && editingId.value) {
        await api.put(`/api/rewrite/${editingId.value}`, payload)
        ElMessage.success('规则更新成功')
      } else {
        await api.post('/api/rewrite', payload)
        ElMessage.success('规则创建成功')
      }
      dialogVisible.value = false
      fetchRules()
    } catch (error: any) {
      const message = error.response?.data?.message || '操作失败'
      ElMessage.error(message)
    } finally {
      submitting.value = false
    }
  })
}

async function submitBatchForm() {
  if (!batchFormRef.value) return

  await batchFormRef.value.validate(async (valid) => {
    if (!valid) return

    batchSubmitting.value = true
    try {
      const payload = {
        patterns: batchFormData.patterns,
        match_type: batchFormData.match_type,
        action_type: batchFormData.action_type,
        action_value: batchFormData.action_type === 'block' ? null : batchFormData.action_value || null,
        priority: batchFormData.priority,
        enabled: batchFormData.enabled,
        description: batchFormData.description || null
      }

      const response = await api.post('/api/rewrite/batch', payload)
      ElMessage.success(`成功创建 ${response.data.created} 条规则`)
      batchDialogVisible.value = false
      fetchRules()
    } catch (error: any) {
      const message = error.response?.data?.message || '批量创建失败'
      ElMessage.error(message)
    } finally {
      batchSubmitting.value = false
    }
  })
}

async function toggleEnabled(rule: RewriteRule) {
  try {
    await api.put(`/api/rewrite/${rule.id}`, { enabled: rule.enabled })
    ElMessage.success(rule.enabled ? '规则已启用' : '规则已禁用')
  } catch (error: any) {
    rule.enabled = !rule.enabled
    ElMessage.error(error.response?.data?.message || '操作失败')
  }
}

async function confirmDelete(rule: RewriteRule) {
  try {
    await ElMessageBox.confirm(
      `确定要删除规则 "${rule.pattern}" 吗？`,
      '确认删除',
      {
        confirmButtonText: '删除',
        cancelButtonText: '取消',
        type: 'warning'
      }
    )
    await api.delete(`/api/rewrite/${rule.id}`)
    ElMessage.success('规则删除成功')
    fetchRules()
  } catch (error: any) {
    if (error !== 'cancel') {
      ElMessage.error(error.response?.data?.message || '删除失败')
    }
  }
}

onMounted(() => {
  fetchRules()
})
</script>

<style scoped>
.rewrite-rules {
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

.header-actions {
  display: flex;
  gap: 12px;
}

.form-hint {
  font-size: 12px;
  color: #909399;
  margin-top: 4px;
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

.pattern-text {
  font-family: 'Monaco', 'Menlo', monospace;
  font-size: 13px;
  color: #303133;
}

.action-value {
  font-family: 'Monaco', 'Menlo', monospace;
  font-size: 13px;
  color: #606266;
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

  .action-btn {
    padding: 12px;
  }
}
</style>

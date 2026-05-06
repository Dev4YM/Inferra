{{- define "inferra.name" -}}
{{- .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "inferra.fullname" -}}
{{- printf "%s-%s" .Release.Name (include "inferra.name" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "inferra.serviceAccountName" -}}
{{- if .Values.serviceAccount.name -}}
{{- .Values.serviceAccount.name -}}
{{- else if .Values.serviceAccount.create -}}
{{- include "inferra.fullname" . -}}
{{- else -}}
default
{{- end -}}
{{- end -}}


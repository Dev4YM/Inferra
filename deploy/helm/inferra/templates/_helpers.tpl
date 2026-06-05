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

{{- define "inferra.authSecretName" -}}
{{- if .Values.auth.existingSecret -}}
{{- .Values.auth.existingSecret -}}
{{- else if .Values.auth.secretName -}}
{{- .Values.auth.secretName -}}
{{- else -}}
{{- printf "%s-auth" (include "inferra.fullname" .) -}}
{{- end -}}
{{- end -}}

{{- define "inferra.mountAuthSecret" -}}
{{- if and .Values.server.authTokenEnv (or .Values.auth.existingSecret .Values.auth.secretName .Values.auth.createSecret) -}}
true
{{- end -}}
{{- end -}}


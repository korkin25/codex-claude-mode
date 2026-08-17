# Repository instructions

## Telegram merge reports

- Executor никогда не отправляет Telegram-отчёты перед commit, после
  push или по промежуточному CI. Отсутствие Telegram route не блокирует
  edit, commit, push или candidate handoff.
- Ровно один фактический merge-отчёт отправляет root/controller только
  после того, как hosting provider подтвердил merge ветки и весь
  обязательный post-merge CI успешно завершился на exact merge SHA.
- Pending, failed, cancelled или непроверенный post-merge CI не является
  основанием для Telegram-сообщения. Correction разрешена только для
  исправления уже отправленного merge-отчёта.
- Не включай в отчёты токены, учётные данные, private payload и другие
  секреты; адрес должен прийти только из активной private root/user
  инструкции.

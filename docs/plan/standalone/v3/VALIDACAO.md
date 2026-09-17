# Repetição e limites

Executar os comandos de EXECUCAO.md num diretório privado. Validadores, modelos e sonda Git usam somente fixtures; não modificam o repositório do usuário. Leia os scripts antes de executar arquivos vindos de terceiros. Scripts de render/validação/modelos/sonda usam Python stdlib e Git local; demonstração criptográfica exige cryptography instalado, sem instalação/rede automática. Ausência de dependência é setup_error e não passa.

`render_plan.py --root .` regenera vistas. `validate_plan.py` compara arquivos completos, regras/bindings, aliases e assertions. Mudanças no documento canônico exigem recalcular digests via `refresh_contracts.py`, rever consumidores e executar a bateria; o script de atualização não aprova semanticamente a revisão.

`test_plan_validator.py` contém controles positivos, ataques históricos relevantes e mutações novas; assertion removida, ambiente faltante, protocolo ausente, freeze/owner/capability errados precisam falhar. `check_protocol_models.py` é exploração limitada de referência, não modelo completo de SO/Git/SQLite. `probe_git_managed.py` é sonda nativa do candidato gerenciado com checkout concorrente e SQLite; não executa Hugit. `test_qualification.py` executa subprocesso real de fixture, assina evidencia com chaves efêmeras e verifica A/B/C — não conta como produto qualificado.

O verifier de hashes confere arquivos distribuídos, não suficiência de requisitos nem autenticidade externa. `validation/` armazena ensaios desta entrega; `evidence-ledger.json` fica sem testes de produto. Os modelos preservam controles inseguros: all-green sem contraexemplo não é evidência de que o ataque estava representado.

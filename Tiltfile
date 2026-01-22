# Tilt-based workflow for the local coding loop

docker_compose('docker-compose.yml')

# Generate .env.local based on host RAM and architecture
local_resource(
    'select-model',
    'python3 scripts/select_model.py --env .env.local',
    deps=['scripts/select_model.py'],
)

# Pull the selected model once Ollama is running
local_resource(
    'ollama-pull',
    'python3 scripts/ollama_pull.py',
    deps=['.env.local', 'scripts/ollama_pull.py'],
    resource_deps=['select-model', 'ollama'],
)

# Wait until all local GGUF files exist before importing
local_resource(
    'wait-models',
    'python3 scripts/wait_models.py',
    deps=['scripts/wait_models.py'],
    resource_deps=['ollama'],
)

# Import local GGUF models into Ollama (auto after downloads finish)
local_resource(
    'ollama-import',
    'python3 scripts/ollama_import.py',
    deps=['scripts/ollama_import.py',
          'modelfiles/qwen2.5-coder-7b-instruct-q4km',
          'modelfiles/deepseek-coder-6.7b-instruct-q4km',
          'modelfiles/starcoder2-7b-q4km',
          'modelfiles/glm-4-9b-chat-q4k'],
    resource_deps=['wait-models'],
)

# Manual Aider session button in Tilt UI
local_resource(
    'aider',
    'docker compose run --rm aider',
    resource_deps=['ollama'],
    trigger_mode=TRIGGER_MODE_MANUAL,
)

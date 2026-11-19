// Links and commands remain available without JavaScript or clipboard access.
for (const button of document.querySelectorAll('[data-copy]')) {
  const commands = document.getElementById(button.dataset.copy);
  const status = button.closest('article, .setup-panel')?.querySelector('.copy-status');
  if (!commands || !status || !navigator.clipboard?.writeText) continue;
  button.hidden = false;
  button.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(commands.textContent.trim());
      status.textContent = 'Copied.';
    } catch {
      status.textContent = 'Copy unavailable. Select the command above to copy it.';
    }
  });
}

const architecture = document.querySelector('#linux-architecture');
if (architecture) {
  architecture.closest('label').hidden = false;
  const showArchitecture = () => {
    for (const group of document.querySelectorAll('[data-arch]')) {
      group.hidden = group.dataset.arch !== architecture.value;
    }
  };
  architecture.addEventListener('change', showArchitecture);
  showArchitecture();
}

const platform = /Mac/.test(navigator.platform) ? 'macos' : /Linux/.test(navigator.platform) ? 'linux' : null;
const suggested = platform && document.querySelector(`[data-platform="${platform}"]`);
if (suggested) {
  suggested.classList.add('suggested');
  suggested.querySelector('.platform-hint').textContent = platform === 'macos'
    ? 'macOS detected. This build requires Apple Silicon.'
    : 'Linux detected. Choose your computer’s architecture.';
}

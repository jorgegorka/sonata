const copyButton = document.querySelector('.copy-button');
const commands = document.querySelector('#setup-commands');
const copyStatus = document.querySelector('.copy-status');

// The setup commands and links remain usable without JavaScript or clipboard access.
if (copyButton && commands && copyStatus && navigator.clipboard?.writeText) {
  copyButton.hidden = false;
  copyButton.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(commands.textContent.trim());
      copyStatus.textContent = 'Commands copied.';
    } catch {
      copyStatus.textContent = 'Copy unavailable. Select the commands above to copy them.';
    }
  });
}

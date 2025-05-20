let ws = null;

const toast = document.getElementById('toast');

function showToast(message, isError = false) {
    toast.textContent = message;
    toast.className = `fixed bottom-4 right-4 p-4 rounded-lg shadow-lg text-white ${isError ? 'bg-red-500' : 'bg-green-500'}`;
    toast.classList.remove('hidden');
    setTimeout(() => toast.classList.add('hidden'), 3000);
}

function validateUrl(url) {
    try {
        new URL(url);
        return true;
    } catch {
        return false;
    }
}


async function fetchStatus() {
    try {
        const response = await fetch('/api/status');
        if (!response.ok) throw new Error('Failed to fetch status or no device selected');
        const data = await response.json();
        document.getElementById('now-playing').textContent = 
            `${data.now_playing?.artist || 'Unknown'} - ${data.now_playing?.track || 'Unknown'}`;
        document.getElementById('volume').textContent = data.volume?.actual_volume || 'Unknown';
        document.getElementById('volume-slider').value = data.volume?.actual_volume || 20;
        document.getElementById('volume-display').textContent = data.volume?.actual_volume || 20;
    } catch (error) {
        showToast(error.message, true);
    }
}

async function discoverDevices() {
    try {
        const response = await fetch('/api/discover');
        if (!response.ok) throw new Error('Failed to discover devices');
        const devices = await response.json();
        const select = document.getElementById('device-select');
        select.innerHTML = '<option value="">Select a device</option>';
        devices.forEach(device => {
            const option = document.createElement('option');
            option.value = device.hostname;
            option.textContent = `${device.realname} ${device.hostname} (${device.ip}:${device.port})`;
            select.appendChild(option);
        });
        if (devices.length > 0) {
            select.value = devices[0].hostname;
            await selectDevice();
            showToast(`${devices.length} device(s) found`);
        } else {
            showToast('No devices found', true);
        }
    } catch (error) {
        showToast(error.message, true);
    }
}

async function selectDevice() {
    const hostname = document.getElementById('device-select').value;
    if (!hostname) return;
    try {
        const response = await fetch('/api/select_device', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ url: hostname })
        });
        if (!response.ok) throw new Error('Failed to select device');
        showToast(`Selected device: ${hostname}`);
        reconnectWebSocket();
        fetchStatus();
    } catch (error) {
        showToast(error.message, true);
    }
}

async function setManualHostname() {
    const hostname = document.getElementById('manual-hostname').value.trim();
    if (!hostname) {
        showToast('Please enter a hostname', true);
        return;
    }
    try {
        const response = await fetch('/api/select_device', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ url: hostname })
        });
        if (!response.ok) throw new Error('Failed to set manual hostname');
        const select = document.getElementById('device-select');
        const option = document.createElement('option');
        option.value = hostname;
        option.textContent = `${hostname} (Manual)`;
        select.appendChild(option);
        select.value = hostname;
        showToast(`Set manual hostname: ${hostname}`);
        reconnectWebSocket();
        fetchStatus();
    } catch (error) {
        showToast(error.message, true);
    }
}

async function playPreset(preset) {
    if (!/^[1-6]$/.test(preset)) {
        showToast('Preset must be 1-6', true);
        return;
    }
    try {
        const response = await fetch('/api/preset', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ url: preset })
        });
        if (!response.ok) throw new Error('Failed to play preset or no device selected');
        showToast(`Playing Preset ${preset}`);
    } catch (error) {
        showToast(error.message, true);
    }
}

async function setVolume(volume) {
    const vol = parseInt(volume);
    if (isNaN(vol) || vol < 0 || vol > 100) {
        showToast('Volume must be 0-100', true);
        return;
    }
    document.getElementById('volume-display').textContent = vol;
    try {
        const response = await fetch('/api/volume', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ url: volume })
        });
        if (!response.ok) throw new Error('Failed to set volume or no device selected');
        showToast(`Volume set to ${volume}`);
    } catch (error) {
        showToast(error.message, true);
    }
}

async function playRadio() {
    const url = document.getElementById('radio-url').value;
    if (!validateUrl(url)) {
        showToast('Invalid radio URL', true);
        return;
    }
    try {
        const response = await fetch('/api/radio', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ url })
        });
        if (!response.ok) throw new Error('Failed to play radio or no device selected');
        showToast('Playing radio stream');
    } catch (error) {
        showToast(error.message, true);
    }
}

async function playYouTube() {
    const url = document.getElementById('youtube-url').value;
    if (!validateUrl(url) || !url.includes('youtube.com')) {
        showToast('Invalid YouTube URL', true);
        return;
    }
    try {
        const response = await fetch('/api/youtube', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ url })
        });
        if (!response.ok) throw new Error('Failed to play YouTube or no device selected');
        showToast('Playing YouTube audio');
    } catch (error) {
        showToast(error.message, true);
    }
}

async function playAction(action) {
    try {
        const response = await fetch('/api/play', {
            method: 'POST',
            headers: { 'Content Type': 'application/json' },
            body: JSON.stringify({ url: action })
        });
        if (!response.ok) throw new Error(`Failed to ${action.toLowerCase()} or no device selected`);
        showToast(`${action} successful`);
    } catch (error) {
        showToast(error.message, true);
    }
}

function reconnectWebSocket() {
    if (ws) {
        ws.close();
    }
    ws = new WebSocket('ws://localhost:3000/ws');
    ws.onmessage = (event) => {
        const data = JSON.parse(event.data);
        if (data.error) {
            showToast(data.error, true);
            return;
        }
        if (data.type === 'now_playing') {
            document.getElementById('now-playing').textContent = 
                `${data.artist || 'Unknown'} - ${data.track || 'Unknown'}`;
        } else if (data.type === 'volume') {
            document.getElementById('volume').textContent = data.volume;
            document.getElementById('volume-slider').value = data.volume;
            document.getElementById('volume-display').textContent = data.volume;
        }
    };
    ws.onclose = () => showToast('WebSocket connection closed', true);
}

// Initial discovery and status
discoverDevices();
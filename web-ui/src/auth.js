const LOGIN_URL    = '/auth/login';
const REGISTER_URL = '/auth/register';
const LOGOUT_URL   = '/auth/logout';
const ME_URL       = '/auth/me';
const CONFIG_URL   = '/auth/config';

// ── Auth state ────────────────────────────────────────────────────────────────

let currentUser = null;

export function getUser() { return currentUser; }
export function setUser(user) { currentUser = user; }
export function isAdmin() { return currentUser?.role === 'admin'; }

// ── API ───────────────────────────────────────────────────────────────────────

export async function fetchMe() {
  const res = await fetch(ME_URL);
  if (res.status === 401) return null;
  if (!res.ok) return null;
  return res.json();
}

export async function login(email, password) {
  const res = await fetch(LOGIN_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email, password }),
  });
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || 'Login failed');
  }
  return res.json();
}

export async function register(email, name, password) {
  const res = await fetch(REGISTER_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email, name, password }),
  });
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || 'Registration failed');
  }
  return res.json();
}

export async function logout() {
  await fetch(LOGOUT_URL, { method: 'POST' });
  currentUser = null;
}

// ── Page init ─────────────────────────────────────────────────────────────────

export function initAuthPages({ onLoginSuccess }) {
  const loginPage    = document.getElementById('page-login');
  const registerPage = document.getElementById('page-register');

  // Hide Google button until we confirm it's enabled server-side
  const googleBtn      = document.getElementById('google-login-btn');
  const googleDivider  = document.getElementById('google-divider');
  googleBtn.hidden    = true;
  googleDivider.hidden = true;
  fetch(CONFIG_URL)
    .then(r => r.ok ? r.json() : null)
    .then(cfg => {
      if (cfg?.google) {
        googleBtn.hidden    = false;
        googleDivider.hidden = false;
      }
    })
    .catch(() => {});

  // Login form
  const loginForm      = document.getElementById('login-form');
  const loginError     = document.getElementById('login-error');
  const toRegisterLink = document.getElementById('to-register-link');

  loginForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    loginError.textContent = '';
    const email    = document.getElementById('login-email').value.trim();
    const password = document.getElementById('login-password').value;
    try {
      await login(email, password);
      const user = await fetchMe();
      if (user) {
        currentUser = user;
        onLoginSuccess(user);
      }
    } catch (err) {
      loginError.textContent = err.message;
    }
  });

  toRegisterLink.addEventListener('click', (e) => {
    e.preventDefault();
    loginPage.hidden = true;
    registerPage.hidden = false;
  });

  googleBtn.addEventListener('click', () => {
    window.location.href = '/auth/google';
  });

  // Register form
  const registerForm    = document.getElementById('register-form');
  const registerError   = document.getElementById('register-error');
  const toLoginLink     = document.getElementById('to-login-link');

  registerForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    registerError.textContent = '';
    const email    = document.getElementById('register-email').value.trim();
    const name     = document.getElementById('register-name').value.trim();
    const password = document.getElementById('register-password').value;
    const confirm  = document.getElementById('register-password-confirm').value;
    if (password.length < 8) {
      registerError.textContent = 'Password must be at least 8 characters.';
      return;
    }
    if (password !== confirm) {
      registerError.textContent = 'Passwords do not match.';
      return;
    }
    try {
      await register(email, name, password);
      const user = await fetchMe();
      if (user) {
        currentUser = user;
        onLoginSuccess(user);
      }
    } catch (err) {
      registerError.textContent = err.message;
    }
  });

  toLoginLink.addEventListener('click', (e) => {
    e.preventDefault();
    registerPage.hidden = true;
    loginPage.hidden = false;
  });
}

export function showLoginPage() {
  document.getElementById('page-login').hidden = false;
  document.getElementById('page-register').hidden = true;
}

export function hideAuthPages() {
  document.getElementById('page-login').hidden = true;
  document.getElementById('page-register').hidden = true;
}

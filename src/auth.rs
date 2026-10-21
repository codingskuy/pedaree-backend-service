//! Autentikasi: JWT akses 15 menit (audiens `pedaree`, Ed25519) + refresh token 7 hari
//! sekali pakai dengan reuse-detection. Pola mengikuti `docs/auth.md` di
//! `lumbung-backend-service`; ringkasan Pedaree ada di `docs/auth.md` repo ini.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{FromRequestParts, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::routing::post;
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::OnceLock;

use crate::config::Config;
use crate::error::{AppError, JsonBaku};
use crate::state::AppState;

/// Audiens tunggal token akses Pedaree; token audiens lain ditolak.
pub const AUDIENS: &str = "pedaree";
pub const AKSES_DETIK: i64 = 15 * 60;
pub const REFRESH_HARI: i64 = 7;

#[derive(Debug, Serialize, Deserialize)]
struct Klaim {
    iss: String,
    sub: String,
    aud: String,
    /// Rumah tangga pemilik data.
    hh: String,
    exp: i64,
    iat: i64,
    jti: String,
}

/// Pasangan kunci Ed25519 untuk menandatangani dan memverifikasi JWT akses.
pub struct KunciJwt {
    enkode: EncodingKey,
    dekode: DecodingKey,
    penerbit: String,
}

impl KunciJwt {
    pub fn dari_config(config: &Config) -> Result<Self, AppError> {
        let enkode = EncodingKey::from_ed_pem(config.jwt_private_pem.nilai().as_bytes())
            .map_err(|e| AppError::internal(format!("JWT_ED25519_PRIVATE tidak valid: {e}")))?;
        let dekode = DecodingKey::from_ed_pem(config.jwt_public_pem.as_bytes())
            .map_err(|e| AppError::internal(format!("JWT_ED25519_PUBLIC tidak valid: {e}")))?;
        Ok(Self {
            enkode,
            dekode,
            penerbit: config.jwt_issuer.clone(),
        })
    }

    fn terbitkan(&self, pengguna_id: &str, household_id: &str) -> Result<String, AppError> {
        let sekarang = Utc::now().timestamp();
        let klaim = Klaim {
            iss: self.penerbit.clone(),
            sub: pengguna_id.to_owned(),
            aud: AUDIENS.to_owned(),
            hh: household_id.to_owned(),
            exp: sekarang + AKSES_DETIK,
            iat: sekarang,
            jti: ulid::Ulid::new().to_string(),
        };
        encode(&Header::new(Algorithm::EdDSA), &klaim, &self.enkode)
            .map_err(|e| AppError::internal(format!("gagal menandatangani JWT: {e}")))
    }

    fn verifikasi(&self, token: &str) -> Result<Klaim, AppError> {
        let mut aturan = Validation::new(Algorithm::EdDSA);
        aturan.set_audience(&[AUDIENS]);
        aturan.set_issuer(&[self.penerbit.as_str()]);
        aturan.set_required_spec_claims(&["exp", "aud", "iss", "sub"]);
        aturan.leeway = 5;
        decode::<Klaim>(token, &self.dekode, &aturan)
            .map(|d| d.claims)
            .map_err(|_| AppError::tidak_sah("Token tidak sah atau kedaluwarsa"))
    }
}

/// Identitas penelepon terautentikasi (ekstraktor Axum).
///
/// Memerlukan `Authorization: Bearer <JWT akses>` beraudiens `pedaree` dan header
/// `X-Household-Id` yang sama dengan klaim `hh` pada token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pengguna {
    pub id: String,
    pub household_id: String,
}

impl FromRequestParts<AppState> for Pengguna {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| AppError::tidak_sah("Header Authorization Bearer wajib"))?;
        let klaim = state.kunci.verifikasi(token)?;

        let household = parts
            .headers
            .get("x-household-id")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError::validasi("Header X-Household-Id wajib", Some("X-Household-Id"))
            })?;
        if household != klaim.hh {
            return Err(AppError::TidakSah(crate::error::Detail::baru(
                "HOUSEHOLD_TIDAK_COCOK",
                "X-Household-Id tidak sesuai dengan token",
                Some("X-Household-Id"),
            )));
        }
        Ok(Self {
            id: klaim.sub,
            household_id: klaim.hh,
        })
    }
}

// ---------------------------------------------------------------------------
// Kata sandi

/// Hash argon2id berformat PHC.
pub fn hash_sandi(sandi: &str, argon: &Argon2<'_>) -> Result<String, AppError> {
    let garam = SaltString::generate(&mut OsRng);
    argon
        .hash_password(sandi.as_bytes(), &garam)
        .map(|h| h.to_string())
        .map_err(|e| AppError::internal(format!("gagal hash kata sandi: {e}")))
}

/// Hash palsu untuk menyamakan waktu respons saat pengguna tidak ditemukan.
fn hash_palsu() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        hash_sandi("bukan-kata-sandi-sungguhan", &Argon2::default()).unwrap_or_default()
    })
}

async fn verifikasi_sandi(hash: String, sandi: String) -> bool {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash)
            .map(|h| {
                Argon2::default()
                    .verify_password(sandi.as_bytes(), &h)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// Membuat pengguna (pendaftaran belum ada di kontrak v1.0.0; dipakai provisioning dan uji).
pub async fn buat_pengguna(
    db: &PgPool,
    household_id: &str,
    id_pengguna: &str,
    kata_sandi: &str,
    argon: &Argon2<'_>,
) -> Result<String, AppError> {
    let id = format!("usr_{}", ulid::Ulid::new());
    let hash = hash_sandi(kata_sandi, argon)?;
    sqlx::query(
        "INSERT INTO pengguna (id, household_id, id_pengguna, kata_sandi_hash)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(&id)
    .bind(household_id)
    .bind(id_pengguna.trim())
    .bind(hash)
    .execute(db)
    .await?;
    Ok(id)
}

// ---------------------------------------------------------------------------
// Refresh token

fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn token_acak() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("rt_{}", URL_SAFE_NO_PAD.encode(bytes))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenPasangan {
    pub token_akses: String,
    pub token_refresh: String,
    pub kedaluwarsa_detik: i64,
    pub audiens: &'static str,
}

async fn terbitkan_pasangan(
    tx: &mut Transaction<'_, Postgres>,
    kunci: &KunciJwt,
    pengguna_id: &str,
    household_id: &str,
    keluarga_id: &str,
    id_perangkat: Option<&str>,
    sekarang: DateTime<Utc>,
) -> Result<TokenPasangan, AppError> {
    let token_refresh = token_acak();
    sqlx::query(
        "INSERT INTO refresh_token (id, pengguna_id, keluarga_id, token_hash, id_perangkat, kedaluwarsa_pada)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(format!("rtk_{}", ulid::Ulid::new()))
    .bind(pengguna_id)
    .bind(keluarga_id)
    .bind(hash_token(&token_refresh))
    .bind(id_perangkat)
    .bind(sekarang + Duration::days(REFRESH_HARI))
    .execute(&mut **tx)
    .await?;
    Ok(TokenPasangan {
        token_akses: kunci.terbitkan(pengguna_id, household_id)?,
        token_refresh,
        kedaluwarsa_detik: AKSES_DETIK,
        audiens: AUDIENS,
    })
}

// ---------------------------------------------------------------------------
// Handler

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginRequest {
    id_pengguna: String,
    kata_sandi: String,
    id_perangkat: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct BarisPengguna {
    id: String,
    household_id: String,
    kata_sandi_hash: String,
    aktif: bool,
}

async fn login(
    State(state): State<AppState>,
    JsonBaku(req): JsonBaku<LoginRequest>,
) -> Result<Json<TokenPasangan>, AppError> {
    if req.id_pengguna.trim().is_empty() {
        return Err(AppError::validasi(
            "idPengguna wajib diisi",
            Some("idPengguna"),
        ));
    }
    if req.kata_sandi.is_empty() {
        return Err(AppError::validasi(
            "kataSandi wajib diisi",
            Some("kataSandi"),
        ));
    }

    let baris: Option<BarisPengguna> = sqlx::query_as(
        "SELECT id, household_id, kata_sandi_hash, aktif FROM pengguna
          WHERE lower(btrim(id_pengguna)) = lower(btrim($1))",
    )
    .bind(&req.id_pengguna)
    .fetch_optional(&state.db)
    .await?;

    // Verifikasi selalu dijalankan agar waktu respons tidak membocorkan keberadaan akun.
    let (hash, pengguna) = match baris {
        Some(b) => (b.kata_sandi_hash.clone(), Some(b)),
        None => (hash_palsu().to_owned(), None),
    };
    let cocok = verifikasi_sandi(hash, req.kata_sandi).await;
    let pengguna = match pengguna {
        Some(p) if cocok && p.aktif => p,
        _ => return Err(AppError::tidak_sah("ID pengguna atau kata sandi salah")),
    };

    let mut tx = state.db.begin().await?;
    let keluarga = format!("fam_{}", ulid::Ulid::new());
    let pasangan = terbitkan_pasangan(
        &mut tx,
        &state.kunci,
        &pengguna.id,
        &pengguna.household_id,
        &keluarga,
        req.id_perangkat.as_deref(),
        Utc::now(),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(pasangan))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefreshRequest {
    token_refresh: String,
}

#[derive(Debug, sqlx::FromRow)]
struct BarisRefresh {
    id: String,
    pengguna_id: String,
    keluarga_id: String,
    id_perangkat: Option<String>,
    kedaluwarsa_pada: DateTime<Utc>,
    dipakai_pada: Option<DateTime<Utc>>,
    dicabut_pada: Option<DateTime<Utc>>,
}

async fn refresh(
    State(state): State<AppState>,
    JsonBaku(req): JsonBaku<RefreshRequest>,
) -> Result<Json<TokenPasangan>, AppError> {
    if req.token_refresh.trim().is_empty() {
        return Err(AppError::validasi(
            "tokenRefresh wajib diisi",
            Some("tokenRefresh"),
        ));
    }
    let sekarang = Utc::now();
    let mut tx = state.db.begin().await?;

    // FOR UPDATE menserialkan pemakaian token yang sama: permintaan kedua melihat
    // `dipakai_pada` terisi dan jatuh ke jalur reuse.
    let baris: Option<BarisRefresh> = sqlx::query_as(
        "SELECT id, pengguna_id, keluarga_id, id_perangkat, kedaluwarsa_pada, dipakai_pada, dicabut_pada
           FROM refresh_token WHERE token_hash = $1 FOR UPDATE",
    )
    .bind(hash_token(&req.token_refresh))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(baris) = baris else {
        return Err(AppError::tidak_sah("Refresh token tidak sah"));
    };

    if baris.dipakai_pada.is_some() || baris.dicabut_pada.is_some() {
        // Reuse-detection: token lama muncul lagi -> cabut SEMUA refresh milik pengguna.
        sqlx::query(
            "UPDATE refresh_token SET dicabut_pada = $2
              WHERE pengguna_id = $1 AND dicabut_pada IS NULL",
        )
        .bind(&baris.pengguna_id)
        .bind(sekarang)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        tracing::warn!(pengguna = %baris.pengguna_id, keluarga = %baris.keluarga_id,
            "refresh token dipakai ulang; seluruh refresh pengguna dicabut");
        return Err(AppError::TidakSah(crate::error::Detail::baru(
            "REFRESH_DIPAKAI_ULANG",
            "Refresh token sudah dipakai; silakan masuk ulang",
            None,
        )));
    }
    if baris.kedaluwarsa_pada <= sekarang {
        return Err(AppError::TidakSah(crate::error::Detail::baru(
            "REFRESH_KEDALUWARSA",
            "Refresh token kedaluwarsa; silakan masuk ulang",
            None,
        )));
    }

    let pengguna: Option<(String, bool)> =
        sqlx::query_as("SELECT household_id, aktif FROM pengguna WHERE id = $1")
            .bind(&baris.pengguna_id)
            .fetch_optional(&mut *tx)
            .await?;
    let household_id = match pengguna {
        Some((hh, true)) => hh,
        _ => return Err(AppError::tidak_sah("Pengguna tidak aktif")),
    };

    sqlx::query("UPDATE refresh_token SET dipakai_pada = $2 WHERE id = $1")
        .bind(&baris.id)
        .bind(sekarang)
        .execute(&mut *tx)
        .await?;
    let pasangan = terbitkan_pasangan(
        &mut tx,
        &state.kunci,
        &baris.pengguna_id,
        &household_id,
        &baris.keluarga_id,
        baris.id_perangkat.as_deref(),
        sekarang,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(pasangan))
}

pub fn rute() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh))
}

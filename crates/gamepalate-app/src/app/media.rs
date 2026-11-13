use gamepalate_core::ids::AppId;
use gamepalate_steam::{description::Block, images::header_url, transport::SteamError};

use super::Gamepalate;
use crate::{
    capsules::{Capsule, capsule},
    details::GameDetails,
    images::{DecodedImage, ImageKind, LoadImageError},
    log::failed,
};

impl Gamepalate {
    pub async fn image(&self, url: &str, kind: ImageKind) -> Result<Vec<u8>, LoadImageError> {
        self.images.image(url, kind).await
    }

    pub async fn image_without_cache(&self, url: &str) -> Result<Vec<u8>, LoadImageError> {
        self.images.download(url).await
    }

    pub async fn extra_image(&self, url: &str) -> Result<DecodedImage, LoadImageError> {
        self.images.extra(url).await
    }

    pub async fn capsule(&self, id: AppId) -> Result<Capsule, LoadImageError> {
        capsule(id, &self.images, &self.capsules).await
    }

    pub async fn details(&self, id: AppId) -> Result<Option<GameDetails>, SteamError> {
        self.store
            .details(id)
            .await
            .inspect_err(failed("could not read the store details"))
    }

    pub async fn description(&self, id: AppId) -> Result<Option<Vec<Block>>, SteamError> {
        self.store
            .description(id)
            .await
            .inspect_err(failed("could not read the store description"))
    }

    pub fn header_url(&self, id: AppId) -> String {
        header_url(id)
    }

    pub fn prune_images(&self) {
        self.images.prune();
    }

    pub fn image_cache_bytes(&self) -> u64 {
        self.images.bytes()
    }

    pub fn clear_image_cache(&self) {
        self.images.clear();
    }
}

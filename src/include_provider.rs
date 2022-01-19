use crate::BoxedIncludeProviderError;

/// User-supplied include reader
pub trait IncludeProvider {
    type IncludeContext;

    fn get_include(
        &mut self,
        path: &str,
        context: &Self::IncludeContext,
    ) -> Result<(String, Self::IncludeContext), BoxedIncludeProviderError>;
}

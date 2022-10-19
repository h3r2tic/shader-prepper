use crate::BoxedIncludeProviderError;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ResolvedIncludePath(pub String);

pub struct ResolvedInclude<IncludeContext> {
    pub resolved_path: ResolvedIncludePath,
    pub context: IncludeContext,
}

/// User-supplied include reader
pub trait IncludeProvider {
    type IncludeContext;

    fn resolve_path(
        &self,
        path: &str,
        context: &Self::IncludeContext,
    ) -> Result<ResolvedInclude<Self::IncludeContext>, BoxedIncludeProviderError>;

    fn get_include(
        &mut self,
        path: &ResolvedIncludePath,
    ) -> Result<String, BoxedIncludeProviderError>;
}

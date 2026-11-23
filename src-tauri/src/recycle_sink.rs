use std::{cell::Cell, rc::Rc};
use windows::{
    core::{implement, Error, Ref, HRESULT, PCWSTR},
    Win32::{
        Foundation::E_ABORT,
        UI::Shell::{
            IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem,
            TSF_DELETE_RECYCLE_IF_POSSIBLE,
        },
    },
};

#[derive(Default)]
pub struct RecycleResult {
    permitted: Cell<bool>,
    completed: Cell<bool>,
    failure: Cell<Option<HRESULT>>,
}

impl RecycleResult {
    fn permit(&self, flags: u32) -> windows::core::Result<()> {
        if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0 {
            self.failure.set(Some(E_ABORT));
            return Err(Error::from(E_ABORT));
        }
        self.permitted.set(true);
        Ok(())
    }
    fn finish_item(&self, result: HRESULT, recycled_destination: bool) {
        if result.is_err() {
            self.failure.set(Some(result));
        } else if !self.permitted.get() || !recycled_destination {
            self.failure.set(Some(E_ABORT));
        } else {
            self.completed.set(true);
        }
    }
    pub fn check(&self) -> windows::core::Result<()> {
        if let Some(error) = self.failure.get() {
            return Err(Error::from(error));
        }
        if !self.completed.get() {
            return Err(Error::from(E_ABORT));
        }
        Ok(())
    }
}

#[implement(IFileOperationProgressSink)]
struct RecycleOnlySink {
    result: Rc<RecycleResult>,
}

pub fn new_sink() -> (IFileOperationProgressSink, Rc<RecycleResult>) {
    let result = Rc::new(RecycleResult::default());
    (
        RecycleOnlySink {
            result: result.clone(),
        }
        .into(),
        result,
    )
}

#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for RecycleOnlySink_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, result: HRESULT) -> windows::core::Result<()> {
        if result.is_err() {
            self.result.failure.set(Some(result));
        }
        Ok(())
    }
    fn PreDeleteItem(&self, flags: u32, _: Ref<IShellItem>) -> windows::core::Result<()> {
        self.result.permit(flags)
    }
    fn PostDeleteItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        result: HRESULT,
        destination: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.result.finish_item(result, destination.is_some());
        Ok(())
    }
    fn PreRenameItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> windows::core::Result<()> {
        Err(Error::from(E_ABORT))
    }
    fn PostRenameItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        _: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Err(Error::from(E_ABORT))
    }
    fn PreMoveItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Err(Error::from(E_ABORT))
    }
    fn PostMoveItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        _: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Err(Error::from(E_ABORT))
    }
    fn PreCopyItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Err(Error::from(E_ABORT))
    }
    fn PostCopyItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        _: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Err(Error::from(E_ABORT))
    }
    fn PreNewItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> windows::core::Result<()> {
        Err(Error::from(E_ABORT))
    }
    fn PostNewItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        _: &PCWSTR,
        _: u32,
        _: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Err(Error::from(E_ABORT))
    }
    fn UpdateProgress(&self, _: u32, _: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permanent_or_missing_recycle_results_are_rejected_and_item_errors_propagate() {
        let result = RecycleResult::default();
        assert!(result.permit(0).is_err());
        assert!(result.check().is_err());
        let result = RecycleResult::default();
        result
            .permit(TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32)
            .unwrap();
        result.finish_item(HRESULT(0), false);
        assert!(result.check().is_err());
        let result = RecycleResult::default();
        result
            .permit(TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32)
            .unwrap();
        result.finish_item(HRESULT(0), true);
        result.check().unwrap();
        result.finish_item(E_ABORT, false);
        assert!(result.check().is_err());
    }
}

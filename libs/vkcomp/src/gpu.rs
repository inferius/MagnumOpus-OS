//! GPU cast skladace: zarizeni Vulkanu pres Venus, vypocetni pipeline se
//! shaderem `spirv::blend`, povrchy a skladani snimku.
//!
//! Povrch = `VkBuffer` v pameti `HOST_VISIBLE | HOST_COHERENT` s blobem
//! namapovanym do procesu (CPU do nej primo zapisuje pixely) a vlastni
//! sadou deskriptoru (vazba 0 = snimek, 1 = povrch). Snimek je stejny
//! buffer; po `compose` v nem je hotovy obraz.
//!
//! `compose` zaznamena do jednoho command bufferu pro kazdou operaci
//! vazbu sady, push constants a dispatch, mezi nimi barieru na pameti
//! shaderu; na zacatku bariera po zapisech hosta, na konci pred ctenim
//! hostem. Cekani na frontu pres plot jadra (`Venus::wait_queue`).
//!
//! `convert_yuv` prevede snimek YUV 4:2:0 (NV12, I420) do povrchu shaderem
//! `spirv::yuv` (stejne vzorce jako `libs/yuv` na CPU, bit po bitu):
//! bajty YUV jdou do odkladaciho bufferu, jedna sada deskriptoru
//! (0 = povrch, 1 = bajty YUV) se pred kazdym prevodem prepise.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::time::Duration;

use libos::Mapping;
use venus::REPLY;
use venus::client::{Blob, Error, Result, Venus, check};
use venus::cs::Encoder;
use venus::vk::{cmd, *};

use crate::plan::{Draw, plan};

/// Nejvic povrchu (sad deskriptoru).
pub const MAX_SURFACES: u32 = 256;
/// Nejvic operaci v jednom odeslani (proud prikazu ma nejvys 60 KiB).
const DRAWS_PER_SUBMIT: usize = 120;
/// Cekani na GPU.
const FENCE_TIMEOUT: Duration = Duration::from_secs(10);

/// Buffer s pameti a blobem.
struct Buf {
    buffer: VkBuffer,
    memory: VkDeviceMemory,
    blob: Blob,
}

/// Zdrojovy povrch (pixely `stride * h`, krok v pixelech).
pub struct Surface {
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    buf: Buf,
    set: VkDescriptorSet,
}

impl Surface {
    /// Pamet povrchu (zapis pixelu z CPU, `u32` little-endian).
    pub fn map(&self) -> &Mapping {
        &self.buf.blob.map
    }
}

pub struct Compositor {
    v: Venus,
    instance: VkInstance,
    device: VkDevice,
    queue: VkQueue,
    host_type: u32,
    shader: VkShaderModule,
    set_layout: VkDescriptorSetLayout,
    layout: VkPipelineLayout,
    pipeline: VkPipeline,
    /// Prevod YUV: shader, pipeline (stejny layout), sada a odkladaci buffer.
    yuv_shader: VkShaderModule,
    yuv_pipeline: VkPipeline,
    yuv_set: VkDescriptorSet,
    yuv_src: Option<Buf>,
    desc_pool: VkDescriptorPool,
    cmd_pool: VkCommandPool,
    cb: VkCommandBuffer,
    fence: VkFence,
    pub w: u32,
    pub h: u32,
    /// Snimek (`None` jen behem `destroy`).
    frame: Option<Buf>,
    surfaces: BTreeMap<u32, Surface>,
    next_surface: u32,
    /// Jmeno fyzickeho zarizeni (log).
    pub device_name: alloc::string::String,
}

fn err() -> Error {
    Error::Vk(VK_ERROR_INITIALIZATION_FAILED)
}

impl Compositor {
    /// Zarizeni, pipeline a snimek `w` x `h` nad spojenim Venus.
    pub fn new(mut v: Venus, w: u32, h: u32) -> Result<Self> {
        // Instance a prvni fyzicke zarizeni.
        let instance = VkInstance(v.new_id());
        let app = VkApplicationInfo { p_application_name: Some("vkcomp"), api_version: venus::api_version(1, 1, 0), ..Default::default() };
        let info = VkInstanceCreateInfo { p_application_info: Some(&app), ..Default::default() };
        check(v.call(|e| cmd::create_instance(e, REPLY, &info, instance), cmd::decode_create_instance_reply)?.ret)?;
        let pd = VkPhysicalDevice(v.new_id());
        let r = v.call(
            |e| cmd::enumerate_physical_devices(e, REPLY, instance, Some(1), Some(&[pd])),
            cmd::decode_enumerate_physical_devices_reply,
        )?;
        if r.ret != VK_SUCCESS && r.ret != VK_INCOMPLETE {
            return Err(Error::Vk(r.ret));
        }
        let props = v
            .call(|e| cmd::get_physical_device_properties(e, REPLY, pd), cmd::decode_get_physical_device_properties_reply)?
            .p_properties
            .ok_or_else(err)?;
        let device_name = alloc::string::String::from(venus::cs::c_str(&props.device_name));
        // Rodina front s vypocty.
        let n = v
            .call(
                |e| cmd::get_physical_device_queue_family_properties(e, REPLY, pd, Some(0), false),
                cmd::decode_get_physical_device_queue_family_properties_reply,
            )?
            .p_queue_family_property_count
            .unwrap_or(0);
        let fams = v
            .call(
                |e| cmd::get_physical_device_queue_family_properties(e, REPLY, pd, Some(n), true),
                cmd::decode_get_physical_device_queue_family_properties_reply,
            )?
            .p_queue_family_properties
            .unwrap_or_default();
        let family = fams.iter().position(|f| f.queue_flags & VK_QUEUE_COMPUTE_BIT != 0).ok_or_else(err)? as u32;
        let mem = v
            .call(|e| cmd::get_physical_device_memory_properties(e, REPLY, pd), cmd::decode_get_physical_device_memory_properties_reply)?
            .p_memory_properties
            .ok_or_else(err)?;
        let host = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
        let host_type = (0..mem.memory_type_count.min(32))
            .find(|&i| mem.memory_types[i as usize].property_flags & host == host)
            .ok_or(Error::Vk(VK_ERROR_OUT_OF_HOST_MEMORY))?;

        // Zarizeni a fronta (renderer chce casovou osu fronty).
        let device = VkDevice(v.new_id());
        let prio = [1.0f32];
        let qci = [VkDeviceQueueCreateInfo { queue_family_index: family, queue_count: 1, p_queue_priorities: &prio, ..Default::default() }];
        let dci = VkDeviceCreateInfo { queue_create_info_count: 1, p_queue_create_infos: &qci, ..Default::default() };
        check(v.call(|e| cmd::create_device(e, REPLY, pd, &dci, device), cmd::decode_create_device_reply)?.ret)?;
        let queue = VkQueue(v.new_id());
        let timeline = VkDeviceQueueTimelineInfoMESA { ring_idx: 1 };
        let chain = [Next::VkDeviceQueueTimelineInfoMESA(&timeline)];
        let qi = VkDeviceQueueInfo2 { p_next: &chain, flags: 0, queue_family_index: family, queue_index: 0 };
        v.send(|e| cmd::get_device_queue2(e, 0, device, &qi, queue))?;

        // Pipeline: shader, sada (2 storage buffery), push constants.
        let code = spirv::blend::shader();
        let shader = VkShaderModule(v.new_id());
        let smci = VkShaderModuleCreateInfo { flags: 0, code_size: (code.len() * 4) as u64, p_code: &code };
        check(v.call(|e| cmd::create_shader_module(e, REPLY, device, &smci, shader), cmd::decode_create_shader_module_reply)?.ret)?;
        let binding = |b: u32| VkDescriptorSetLayoutBinding {
            binding: b,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            descriptor_count: 1,
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            ..Default::default()
        };
        let bindings = [binding(0), binding(1)];
        let set_layout = VkDescriptorSetLayout(v.new_id());
        let dslci = VkDescriptorSetLayoutCreateInfo { flags: 0, binding_count: 2, p_bindings: &bindings };
        check(
            v.call(|e| cmd::create_descriptor_set_layout(e, REPLY, device, &dslci, set_layout), cmd::decode_create_descriptor_set_layout_reply)?
                .ret,
        )?;
        let ranges = [VkPushConstantRange { stage_flags: VK_SHADER_STAGE_COMPUTE_BIT, offset: 0, size: spirv::blend::PUSH_SIZE }];
        let set_layouts = [set_layout];
        let layout = VkPipelineLayout(v.new_id());
        let plci = VkPipelineLayoutCreateInfo {
            flags: 0,
            set_layout_count: 1,
            p_set_layouts: &set_layouts,
            push_constant_range_count: 1,
            p_push_constant_ranges: &ranges,
        };
        check(v.call(|e| cmd::create_pipeline_layout(e, REPLY, device, &plci, layout), cmd::decode_create_pipeline_layout_reply)?.ret)?;
        let pipeline = VkPipeline(v.new_id());
        let cpci = [VkComputePipelineCreateInfo {
            stage: VkPipelineShaderStageCreateInfo {
                stage: VK_SHADER_STAGE_COMPUTE_BIT,
                module: shader,
                p_name: Some("main"),
                ..Default::default()
            },
            layout,
            base_pipeline_index: -1,
            ..Default::default()
        }];
        check(
            v.call(
                |e| cmd::create_compute_pipelines(e, REPLY, device, VkPipelineCache(0), 1, &cpci, Some(&[pipeline])),
                cmd::decode_create_compute_pipelines_reply,
            )?
            .ret,
        )?;
        // Prevod YUV: druhy shader se stejnym layoutem (push constants
        // prevodu jsou kratsi nez rozsah layoutu).
        const _: () = assert!(spirv::yuv::PUSH_SIZE <= spirv::blend::PUSH_SIZE);
        let yuv_code = spirv::yuv::shader();
        let yuv_shader = VkShaderModule(v.new_id());
        let ysmci = VkShaderModuleCreateInfo { flags: 0, code_size: (yuv_code.len() * 4) as u64, p_code: &yuv_code };
        check(v.call(|e| cmd::create_shader_module(e, REPLY, device, &ysmci, yuv_shader), cmd::decode_create_shader_module_reply)?.ret)?;
        let yuv_pipeline = VkPipeline(v.new_id());
        let ycpci = [VkComputePipelineCreateInfo {
            stage: VkPipelineShaderStageCreateInfo {
                stage: VK_SHADER_STAGE_COMPUTE_BIT,
                module: yuv_shader,
                p_name: Some("main"),
                ..Default::default()
            },
            layout,
            base_pipeline_index: -1,
            ..Default::default()
        }];
        check(
            v.call(
                |e| cmd::create_compute_pipelines(e, REPLY, device, VkPipelineCache(0), 1, &ycpci, Some(&[yuv_pipeline])),
                cmd::decode_create_compute_pipelines_reply,
            )?
            .ret,
        )?;
        // Sady: povrchy + jedna pro prevod YUV.
        let sizes = [VkDescriptorPoolSize { r#type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, descriptor_count: 2 * (MAX_SURFACES + 1) }];
        let desc_pool = VkDescriptorPool(v.new_id());
        let dpci = VkDescriptorPoolCreateInfo {
            flags: VK_DESCRIPTOR_POOL_CREATE_FREE_DESCRIPTOR_SET_BIT,
            max_sets: MAX_SURFACES + 1,
            pool_size_count: 1,
            p_pool_sizes: &sizes,
        };
        check(v.call(|e| cmd::create_descriptor_pool(e, REPLY, device, &dpci, desc_pool), cmd::decode_create_descriptor_pool_reply)?.ret)?;
        let yuv_set = VkDescriptorSet(v.new_id());
        let yuv_layouts = [set_layout];
        let ydsai = VkDescriptorSetAllocateInfo { descriptor_pool: desc_pool, descriptor_set_count: 1, p_set_layouts: &yuv_layouts };
        check(
            v.call(|e| cmd::allocate_descriptor_sets(e, REPLY, device, &ydsai, Some(&[yuv_set])), cmd::decode_allocate_descriptor_sets_reply)?
                .ret,
        )?;
        let cmd_pool = VkCommandPool(v.new_id());
        let pci = VkCommandPoolCreateInfo { flags: VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT, queue_family_index: family };
        check(v.call(|e| cmd::create_command_pool(e, REPLY, device, &pci, cmd_pool), cmd::decode_create_command_pool_reply)?.ret)?;
        let cb = VkCommandBuffer(v.new_id());
        let cai = VkCommandBufferAllocateInfo { command_pool: cmd_pool, level: VK_COMMAND_BUFFER_LEVEL_PRIMARY, command_buffer_count: 1 };
        check(v.call(|e| cmd::allocate_command_buffers(e, REPLY, device, &cai, Some(&[cb])), cmd::decode_allocate_command_buffers_reply)?.ret)?;
        let fence = VkFence(v.new_id());
        check(v.call(|e| cmd::create_fence(e, REPLY, device, &VkFenceCreateInfo { flags: 0 }, fence), cmd::decode_create_fence_reply)?.ret)?;
        let frame = alloc_buf(&mut v, device, host_type, w as u64 * h as u64 * 4)?;
        Ok(Self {
            v,
            instance,
            device,
            queue,
            host_type,
            shader,
            set_layout,
            layout,
            pipeline,
            yuv_shader,
            yuv_pipeline,
            yuv_set,
            yuv_src: None,
            desc_pool,
            cmd_pool,
            cb,
            fence,
            w,
            h,
            frame: Some(frame),
            surfaces: BTreeMap::new(),
            next_surface: 1,
            device_name,
        })
    }

    /// Hotovy snimek (`w * h` pixelu po `compose`).
    pub fn frame(&self) -> Option<&Mapping> {
        self.frame.as_ref().map(|f| &f.blob.map)
    }

    fn frame_buffer(&self) -> Result<VkBuffer> {
        self.frame.as_ref().map(|f| f.buffer).ok_or_else(err)
    }

    pub fn surface(&self, id: u32) -> Option<&Surface> {
        self.surfaces.get(&id)
    }

    /// Novy povrch `w` x `h` (krok = `w`); pixely zapisuje CPU pres `map`.
    pub fn create_surface(&mut self, w: u32, h: u32) -> Result<u32> {
        if w == 0 || h == 0 || self.surfaces.len() as u32 >= MAX_SURFACES {
            return Err(Error::Os(libos::Error::InvalidArgs));
        }
        let buf = alloc_buf(&mut self.v, self.device, self.host_type, w as u64 * h as u64 * 4)?;
        let set = VkDescriptorSet(self.v.new_id());
        let layouts = [self.set_layout];
        let dsai = VkDescriptorSetAllocateInfo { descriptor_pool: self.desc_pool, descriptor_set_count: 1, p_set_layouts: &layouts };
        let device = self.device;
        check(
            self.v
                .call(|e| cmd::allocate_descriptor_sets(e, REPLY, device, &dsai, Some(&[set])), cmd::decode_allocate_descriptor_sets_reply)?
                .ret,
        )?;
        let frame_info = [VkDescriptorBufferInfo { buffer: self.frame_buffer()?, offset: 0, range: VK_WHOLE_SIZE }];
        let src_info = [VkDescriptorBufferInfo { buffer: buf.buffer, offset: 0, range: VK_WHOLE_SIZE }];
        let w0 = VkWriteDescriptorSet {
            dst_set: set,
            dst_binding: 0,
            descriptor_count: 1,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            p_buffer_info: &frame_info,
            ..Default::default()
        };
        let w1 = VkWriteDescriptorSet { dst_binding: 1, p_buffer_info: &src_info, ..w0 };
        let writes = [w0, w1];
        self.v.send(|e| cmd::update_descriptor_sets(e, 0, device, 2, &writes, 0, &[]))?;
        let id = self.next_surface;
        self.next_surface += 1;
        self.surfaces.insert(id, Surface { w, h, stride: w, buf, set });
        Ok(id)
    }

    pub fn destroy_surface(&mut self, id: u32) -> Result<()> {
        let Some(s) = self.surfaces.remove(&id) else { return Ok(()) };
        let (device, pool, set) = (self.device, self.desc_pool, s.set);
        free_buf(&mut self.v, device, s.buf)?;
        self.v.send(|e| cmd::free_descriptor_sets(e, 0, device, pool, 1, &[set]))
    }

    /// Slozi operace (zdola nahoru) do snimku a pocka na GPU. Vraci pocet
    /// dispatchu (operace mimo snimek nebo s neznamym povrchem se vynechaji).
    pub fn compose(&mut self, draws: &[Draw]) -> Result<usize> {
        let mut plans = Vec::with_capacity(draws.len());
        for d in draws {
            let Some(s) = self.surfaces.get(&d.src) else { continue };
            if let Some(p) = plan(d, self.w, self.h, s.w, s.h, s.stride) {
                plans.push((s.set, p));
            }
        }
        for chunk in plans.chunks(DRAWS_PER_SUBMIT) {
            self.submit(chunk)?;
        }
        Ok(plans.len())
    }

    fn submit(&mut self, chunk: &[(VkDescriptorSet, crate::plan::Dispatch)]) -> Result<()> {
        let (cb, layout, pipeline, queue, fence, device) = (self.cb, self.layout, self.pipeline, self.queue, self.fence, self.device);
        let cbs = [cb];
        let submit = [VkSubmitInfo { command_buffer_count: 1, p_command_buffers: &cbs, ..Default::default() }];
        let r = self.v.call(
            |e| {
                cmd::reset_command_buffer(e, 0, cb, 0);
                let begin = VkCommandBufferBeginInfo { flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT, p_inheritance_info: None };
                cmd::begin_command_buffer(e, 0, cb, &begin);
                barrier(e, cb, VK_PIPELINE_STAGE_HOST_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_HOST_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT);
                cmd::cmd_bind_pipeline(e, 0, cb, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline);
                for (i, (set, d)) in chunk.iter().enumerate() {
                    if i > 0 {
                        // Dalsi vrstva cte, co predchozi zapsala.
                        let rw = VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;
                        barrier(e, cb, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, rw);
                    }
                    cmd::cmd_bind_descriptor_sets(e, 0, cb, VK_PIPELINE_BIND_POINT_COMPUTE, layout, 0, 1, &[*set], 0, &[]);
                    let bytes: Vec<u8> = d.push.words().iter().flat_map(|w| w.to_le_bytes()).collect();
                    cmd::cmd_push_constants(e, 0, cb, layout, VK_SHADER_STAGE_COMPUTE_BIT, 0, bytes.len() as u32, &bytes);
                    cmd::cmd_dispatch(e, 0, cb, d.groups.0, d.groups.1, 1);
                }
                barrier(e, cb, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_HOST_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_HOST_READ_BIT);
                cmd::end_command_buffer(e, 0, cb);
                cmd::reset_fences(e, 0, device, 1, &[fence]);
                cmd::queue_submit(e, REPLY, queue, 1, &submit, fence);
            },
            cmd::decode_queue_submit_reply,
        )?;
        check(r.ret)?;
        // Plot fronty (osa 1, VkDeviceQueueTimelineInfoMESA) jako plot jadra.
        let _ = (device, fence);
        self.v.wait_queue(1, FENCE_TIMEOUT)
    }

    /// Prevede snimek YUV (`data`: bajty rovin, offsety a kroky v `push`)
    /// do povrchu `surface` (`w` x `h` z `push`, krok povrchu) a pocka na GPU.
    pub fn convert_yuv(&mut self, surface: u32, data: &[u8], push: spirv::yuv::Push) -> Result<()> {
        let s = self.surfaces.get(&surface).ok_or(Error::Os(libos::Error::NotFound))?;
        if push.w == 0 || push.h == 0 || push.w > s.w || push.h > s.h || push.dst_stride != s.stride {
            return Err(Error::Os(libos::Error::InvalidArgs));
        }
        let dst = s.buf.buffer;
        let need = (data.len() as u64).next_multiple_of(4);
        if self.yuv_src.as_ref().is_none_or(|b| b.blob.size < need) {
            if let Some(old) = self.yuv_src.take() {
                free_buf(&mut self.v, self.device, old)?;
            }
            self.yuv_src = Some(alloc_buf(&mut self.v, self.device, self.host_type, need)?);
        }
        let src = self.yuv_src.as_ref().ok_or_else(err)?;
        src.blob.map.write_bytes(0, data);
        let src_buf = src.buffer;
        let (device, cb, set, pipeline, layout, queue) = (self.device, self.cb, self.yuv_set, self.yuv_pipeline, self.layout, self.queue);
        let dst_info = [VkDescriptorBufferInfo { buffer: dst, offset: 0, range: VK_WHOLE_SIZE }];
        let src_info = [VkDescriptorBufferInfo { buffer: src_buf, offset: 0, range: VK_WHOLE_SIZE }];
        let w0 = VkWriteDescriptorSet {
            dst_set: set,
            dst_binding: 0,
            descriptor_count: 1,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            p_buffer_info: &dst_info,
            ..Default::default()
        };
        let w1 = VkWriteDescriptorSet { dst_binding: 1, p_buffer_info: &src_info, ..w0 };
        let writes = [w0, w1];
        let cbs = [cb];
        let submit = [VkSubmitInfo { command_buffer_count: 1, p_command_buffers: &cbs, ..Default::default() }];
        let groups = (push.w.div_ceil(spirv::yuv::LOCAL), push.h.div_ceil(spirv::yuv::LOCAL));
        let bytes: Vec<u8> = push.words().iter().flat_map(|w| w.to_le_bytes()).collect();
        let r = self.v.call(
            |e| {
                cmd::update_descriptor_sets(e, 0, device, 2, &writes, 0, &[]);
                cmd::reset_command_buffer(e, 0, cb, 0);
                let begin = VkCommandBufferBeginInfo { flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT, p_inheritance_info: None };
                cmd::begin_command_buffer(e, 0, cb, &begin);
                barrier(e, cb, VK_PIPELINE_STAGE_HOST_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_HOST_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT);
                cmd::cmd_bind_pipeline(e, 0, cb, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline);
                cmd::cmd_bind_descriptor_sets(e, 0, cb, VK_PIPELINE_BIND_POINT_COMPUTE, layout, 0, 1, &[set], 0, &[]);
                cmd::cmd_push_constants(e, 0, cb, layout, VK_SHADER_STAGE_COMPUTE_BIT, 0, bytes.len() as u32, &bytes);
                cmd::cmd_dispatch(e, 0, cb, groups.0, groups.1, 1);
                barrier(e, cb, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_HOST_BIT | VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_HOST_READ_BIT | VK_ACCESS_SHADER_READ_BIT);
                cmd::end_command_buffer(e, 0, cb);
                cmd::queue_submit(e, REPLY, queue, 1, &submit, VkFence(0));
            },
            cmd::decode_queue_submit_reply,
        )?;
        check(r.ret)?;
        self.v.wait_queue(1, FENCE_TIMEOUT)
    }

    /// Uvolni vse na GPU (bloby pred pameti) a vrati spojeni.
    pub fn destroy(mut self) -> Result<Venus> {
        for id in self.surfaces.keys().copied().collect::<Vec<_>>() {
            self.destroy_surface(id)?;
        }
        let device = self.device;
        if let Some(b) = self.yuv_src.take() {
            free_buf(&mut self.v, device, b)?;
        }
        if let Some(frame) = self.frame.take() {
            free_buf(&mut self.v, device, frame)?;
        }
        let (yuv_pipeline, yuv_shader) = (self.yuv_pipeline, self.yuv_shader);
        let (fence, cmd_pool, cb, desc_pool, pipeline, layout, set_layout, shader, instance) = (
            self.fence,
            self.cmd_pool,
            self.cb,
            self.desc_pool,
            self.pipeline,
            self.layout,
            self.set_layout,
            self.shader,
            self.instance,
        );
        self.v.send(|e| {
            cmd::destroy_fence(e, 0, device, fence);
            cmd::free_command_buffers(e, 0, device, cmd_pool, 1, &[cb]);
            cmd::destroy_command_pool(e, 0, device, cmd_pool);
            cmd::destroy_descriptor_pool(e, 0, device, desc_pool);
            cmd::destroy_pipeline(e, 0, device, pipeline);
            cmd::destroy_pipeline(e, 0, device, yuv_pipeline);
            cmd::destroy_shader_module(e, 0, device, yuv_shader);
            cmd::destroy_pipeline_layout(e, 0, device, layout);
            cmd::destroy_descriptor_set_layout(e, 0, device, set_layout);
            cmd::destroy_shader_module(e, 0, device, shader);
            cmd::destroy_device(e, 0, device);
            cmd::destroy_instance(e, 0, instance);
        })?;
        Ok(self.v)
    }
}

/// Globalni bariera pameti.
fn barrier(e: &mut Encoder, cb: VkCommandBuffer, src_stage: u32, dst_stage: u32, src: u32, dst: u32) {
    let m = [VkMemoryBarrier { src_access_mask: src, dst_access_mask: dst }];
    cmd::cmd_pipeline_barrier(e, 0, cb, src_stage, dst_stage, 0, 1, &m, 0, &[], 0, &[]);
}

/// Buffer `bytes` v pameti viditelne z hosta s blobem namapovanym do procesu.
fn alloc_buf(v: &mut Venus, device: VkDevice, host_type: u32, bytes: u64) -> Result<Buf> {
    let size = bytes.next_multiple_of(4096);
    let buffer = VkBuffer(v.new_id());
    let bci = VkBufferCreateInfo {
        size,
        usage: VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        ..Default::default()
    };
    check(v.call(|e| cmd::create_buffer(e, REPLY, device, &bci, buffer), cmd::decode_create_buffer_reply)?.ret)?;
    let req = v
        .call(|e| cmd::get_buffer_memory_requirements(e, REPLY, device, buffer), cmd::decode_get_buffer_memory_requirements_reply)?
        .p_memory_requirements
        .ok_or_else(err)?;
    if req.memory_type_bits & (1 << host_type) == 0 {
        return Err(Error::Vk(VK_ERROR_OUT_OF_HOST_MEMORY));
    }
    let alloc_size = req.size.max(size).next_multiple_of(4096);
    let memory = VkDeviceMemory(v.new_id());
    let mai = VkMemoryAllocateInfo { allocation_size: alloc_size, memory_type_index: host_type };
    check(v.call(|e| cmd::allocate_memory(e, REPLY, device, &mai, memory), cmd::decode_allocate_memory_reply)?.ret)?;
    check(v.call(|e| cmd::bind_buffer_memory(e, REPLY, device, buffer, memory, 0), cmd::decode_bind_buffer_memory_reply)?.ret)?;
    let blob = v.create_blob(memory.0, alloc_size)?;
    Ok(Buf { buffer, memory, blob })
}

fn free_buf(v: &mut Venus, device: VkDevice, b: Buf) -> Result<()> {
    let (buffer, memory) = (b.buffer, b.memory);
    v.destroy_blob(b.blob)?;
    v.send(|e| {
        cmd::destroy_buffer(e, 0, device, buffer);
        cmd::free_memory(e, 0, device, memory);
    })
}

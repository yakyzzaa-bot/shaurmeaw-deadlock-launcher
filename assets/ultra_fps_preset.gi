// Ультра FPS конфиг ShaurMeaw (убирает тени, туман, постобработку и визуальный мусор)
"GameInfo"
{
    FileSystem
    {
        SearchPaths
        {
            Game_UILanguage     citadel_*LANGUAGE*
            Game_LowViolence    citadel_lv
            Game                citadel/addons
            Mod                 citadel
            Write               citadel
            Game                citadel
            Mod                 core
            Write               core
            Game                core
        }
    }

    SceneSystem
    {
        // Тени и их атласы не нужны, тени выключены
        FogCachedShadowAtlasWidth                     "0"
        FogCachedShadowAtlasHeight                    "0"
        FogCachedShadowTileSize                       "0"
        CSMCascadeResolution                          "0"
        DefaultShadowTextureWidth                     "0"
        DefaultShadowTextureHeight                    "0"

        // Туман давал светлую дымку на всей карте
        NonTexturedGradientFog                        "0"
        CubemapFog                                    "0"
        VolumetricFog                                 "0"
    }

    ConVars
    {
        // ===== Камера =====
        r_aspectratio                                 "1.75"  // FOV: 1.33 = 70, 1.56 = 75, 1.75 = 80, 2.0 = 85, 2.15 = 90, 2.49 = 100, 3.0 = 110
        citadel_camera_use_vmdl_flatten_horizontal    "false"  // без сглаживания камеры по горизонтали
        citadel_camera_use_vmdl_flatten_vertical      "false"  // и по вертикали
        citadel_camera_wobble_disable                 "true"  // без покачивания камеры
        citadel_melee_shake_amplitude                 "0"  // без тряски от ближнего боя
        citadel_melee_shake_duration                  "0"

        // ===== Дальность прорисовки =====
        r_propsmaxdist                                "2500"  // как далеко видно коробки и мелкие предметы
        r_size_cull_threshold                         "0.8"  // объекты меньше 0.8% экрана не рисуются; больше ставить не надо, пропадают далёкие герои
        r_size_cull_threshold_shadow                  "1.0"
        r_farz                                        "4500"  // дальняя граница; -1 = как задано картой
        r_mapextents                                  "4500"
        citadel_portrait_world_renderer_off           "0"  // 1 = в лавке не рисуется модель героя

        // ===== Декали и мелочи =====
        r_drawdecals                                  "0"  // без следов на стенах и полу; закомментируй, если хочешь видеть, например, удар Lash или ульту Warden
        r_character_decal_resolution                  "128"
        citadel_per_weapon_per_surface_impact_effects "false"  // одинаковые простые попадания вместо отдельных для каждой поверхности
        r_citadel_cloak_refract_amount                "0"  // без «жидкого» искажения у невидимых героев
        r_citadel_cloak_blur_amount                   "0"

        // ===== Тени =====
        r_citadel_disable_npr_light                   "true"
        r_citadel_shadow_quality                      "1"
        r_shadows                                     "0"
        r_citadel_shadow_caching                      "0"
        r_citadel_distancefield_shadows               "0"
        sparseshadowtree_enable_rendering             "0"
        sparseshadowtree_disable_add_layers           "1"
        sparseshadowtree_disable_for_viewmodel        "1"
        sparseshadowtree_leaf_precision_viewmodel     "0"
        lb_precomputed_shadowmap_enable               "0"
        r_citadel_gpu_preview_baked_shadows           "0"
        r_citadel_gpu_preview_denoise                 "0"
        r_citadel_gpu_preview_denoise_shadow_passes   "0"
        r_citadel_gpu_preview_denoise_passes          "0"
        lb_csm_cascade_size_override                  "1"
        lb_csm_draw_alpha_tested                      "0"
        lb_csm_draw_translucent                       "0"
        lb_dynamic_shadow_resolution                  "0"
        lb_dynamic_shadow_resolution_base             "16"
        lb_dynamic_shadow_resolution_quantization     "32"
        lb_dynamic_shadow_penumbra                    "0"
        lb_sun_csm_size_cull_threshold_texels         "100"
        lb_barnlight_shadowmap_scale                  "0.5"
        lb_enable_shadow_casting                      "0"
        lb_timesliced_shadows_dynamic_size            "0"
        lb_shadow_map_cull_empty_mixed                "1"
        sc_disable_spotlight_shadows                  "1"
        r_citadel_gpu_culling_shadows                 "1"
        r_citadel_sun_shadow_slope_scale_depth_bias   "0"
        sc_instanced_mesh_size_cull_bias_shadow       "10"
        csm_max_num_cascades_override                 "1"
        csm_max_shadow_dist_override                  "1000"
        csm_viewmodel_max_shadow_dist                 "1"
        csm_viewmodel_max_visible_dist                "1"
        // Атлас теней. Меньше 1024 не ставь: в лавке и хабе у героя есть лампы с тенями, и он начинает мигать.
        // На FPS в матче не влияет: на картах ламп с тенями нет.
        lb_shadow_texture_width_override              "1024"
        lb_shadow_texture_height_override             "1024"

        // ===== Туман и небо =====
        r_enable_volume_fog                           "0"
        r_enable_gradient_fog                         "0"
        r_enable_cubemap_fog                          "0"
        r_citadel_fog_quality                         "0"
        fog_enable                                    "0"
        volume_fog_density_scale                      "0"
        volume_fog_enable_jitter                      "0"
        volume_fog_temporal_filter                    "0"
        volume_fog_intermediate_textures_hdr          "0"
        r_citadel_distancefield_blur                  "0"
        r_drawskybox                                  "0"
        r_draw3dskybox                                "0"
        r_monitor_3dskybox                            "0"

        // ===== Свет =====
        r_directlighting                              "0"
        r_arealights                                  "0"
        r_directional_lightmaps                       "0"
        r_lightmap_size                               "1"
        r_lightmap_size_directional_irradiance        "0"
        r_light_flickering_enabled                    "0"  // без мерцающих ламп
        r_rendersun                                   "0"
        lb_ssss_samples                               "1"  // без подповерхностного рассеивания на коже, заметно экономит GPU
        mat_max_lighting_complexity                   "0"
        r_dashboard_render_quality                    "0"
        r_environment_map_roughness_range             "0.01 0.01"  // почти без отражений на карте
        lb_cubemap_normalization_roughness_begin      "0.01"
        lb_enable_lights                              "0"
        lb_enable_dynamic_lights                      "0"
        lb_enable_stationary_lights                   "0"
        lb_enable_sunlight                            "0"
        lb_enable_baked_shadows                       "1"
        lb_enable_fog_mixed_shadows                   "0"
        lb_mixed_shadows                              "0"
        lb_max_visible_barn_lights_override           "1"
        lb_max_visible_envmaps_override               "-1"
        lb_enable_binning                             "0"
        cl_retire_low_priority_lights                 "1"

        // ===== Затенение (AO) =====
        r_ssao                                        "0"
        r_citadel_ssao_quality                        "0"
        r_ssao_blur                                   "0"
        r_citadel_distancefield_ao_quality            "0"
        r_aoproxy_cull_dist                           "0.01"

        // ===== Постобработка =====
        // Выключена вся: без цветокоррекции, HDR и автоэкспозиции. Карта тёмная и ровная, без засветов.
        r_postprocess_enable                          "0"
        sc_hdr_enabled_override                       "0"
        r_effects_bloom                               "0"
        r_post_bloom                                  "0"
        r_bloom_tent_filter_radius                    "0"
        mat_tonemap_bloom_scale                       "0"
        r_depth_of_field                              "0"
        r_citadel_depthoffield_enable                 "0"
        r_citadel_upscaling                           "0"  // без апскейла (DLSS/FSR); играешь с ним, закомментируй
        mat_colcorrection_disableentities             "1"

        // ===== Distance field (тяжёлая штука для GPU) =====
        r_distancefield_enable                        "0"
        r_citadel_distancefield_farfield_enable       "0"
        r_citadel_distancefield_down_sample           "0"

        // ===== Эффекты и частицы =====
        r_particle_max_detail_level                   "0"  // только самые простые версии эффектов
        r_particle_max_draw_distance                  "3333"
        r_particle_max_size_cull                      "256"
        r_particle_max_texture_layers                 "4"
        r_particle_mixed_resolution_viewstart         "0"  // эффекты в пониженном разрешении
        r_particle_depth_feathering                   "0"  // без мягких краёв у эффектов
        r_particle_cables_render                      "1"
        r_particle_cables_cast_shadows                "0"
        r_particle_skip_postsim                       "1"
        r_particle_fixedrandomseeds                   "1"
        r_particle_newinput                           "1"
        r_update_particles_on_render_only_frames      "1"
        r_physics_particle_op_spawn_scale             "0"
        cl_particle_fallback_base                     "1"  // под нагрузкой игра раньше переходит на упрощённые эффекты
        cl_particle_fallback_multiplier               "2"
        cl_particle_sim_fallback_threshold_ms         "1"
        cl_particle_sim_fallback_base_multiplier      "10"
        cl_max_particle_pvs_aabb_edge_length          "50"
        cl_particle_batch_mode                        "1"
        particle_cluster_nodraw                       "1"
        particle_cluster_use_collision_hulls          "false"
        r_muzzleflashbrightness                       "0"  // без вспышек света от выстрелов
        r_citadel_disable_particle_lights             "true"  // эффекты не светят на стены и героев (засветы, моргание); новая настройка Valve
        cl_impacteffects                              "0"
        cl_show_splashes                              "0"
        fx_drawmetalspark                             "0"  // без искр при попадании в металл
        r_impacts_alt_orientation                     "0"
        func_break_max_pieces                         "0"  // без обломков от ломающихся предметов
        props_break_max_pieces_perframe               "1"
        violence_ablood                               "0"  // без крови
        violence_agibs                                "0"
        violence_hblood                               "0"
        violence_hgibs                                "0"
        citadel_bullet_shot_offset_fade_time          "0"

        // ===== Трава, ветер, мелкий мусор =====
        r_grass_quality                               "0"
        r_grass_allow_flattening                      "0"
        r_grass_vertex_lighting                       "0"
        r_grass_start_fade                            "0"
        r_grass_end_fade                              "0"
        sc_clutter_enable                             "0"  // без камешков и мелкого мусора на земле
        r_world_wind_strength                         "0"
        r_world_wind_frequency_grass                  "0"
        r_world_wind_frequency_trees                  "0"

        // ===== Волосы =====
        // Сами волосы оставлены: без них часть героев выглядит лысыми и сломанными
        r_hair_ao                                     "0"
        r_hair_indirect_transmittance                 "0"
        r_hair_shadowtile                             "0"
        r_hair_wind_motion_scale                      "0"
        r_hair_wind_noise                             "0"
        r_force_thick_hair                            "0"

        // ===== Модели, LOD, анимации =====
        skeleton_instance_lod_optimization            "1"
        r_morphing_enabled                            "0"  // без мимики
        r_smooth_morph_normals                        "0"
        scene_clientflex                              "0"
        enable_boneflex                               "0"
        cloth_sim_on_tick                             "0"
        r_strip_invisible_during_sceneobject_update   "1"
        sc_instanced_mesh_lod_bias                    "3"  // деревья, камни и т.п. рисуются проще
        sc_instanced_mesh_size_cull_bias              "3"
        sc_instanced_mesh_motion_vectors              "0"
        sc_instanced_mesh_opaque_fade                 "0"
        sc_allow_dithered_lod                         "0"
        sc_fade_distance_scale_override               "100"
        sc_force_materials_batchable                  "1"  // меньше вызовов отрисовки
        animated_material_attributes                  "0"
        anim_resource_validate_on_load                "0"

        // ===== Рэгдоллы и физика =====
        cl_disable_ragdolls                           "1"  // убитые сразу исчезают
        cl_ragdoll_default_scale                      "0"
        cl_ragdoll_limit                              "-1"  // по отзывам, при 0 пропадает индикатор ульты Doorman; тела убитых всё равно выключены (cl_disable_ragdolls)
        g_ragdoll_maxcount                            "0"
        g_ragdoll_important_maxcount                  "0"
        cl_batch_entity_list_ops_during_latch         "1"
        rope_collide                                  "0"
        phys_cull_internal_mesh_contacts              "1"
        cl_simulate_dormant_entities                  "0"
        r_drawropes                                   "0"
        r_ropetranslucent                             "0"

        // ===== Текстуры =====
        // «Качество текстур» в меню игры поставь на «Низкое». Тогда мир мыльный, а герои нормальные:
        // их текстуры крупные и после занижения остаются ~512–1024px.
        r_texture_lod_scale                           "1"
        r_fallback_texture_lod_scale                  "16"  // дальние здания и упрощённая геометрия ещё мыльнее
        r_texture_stream_max_resolution               "1024"  // потолок 1024px
        r_texturefilteringquality                     "0"  // билинейная: пол и стены под углом размываются, герои почти нет
        r_citadel_fsr_enable_mip_bias                 "8"
        v8_maximum_heap_size_mb                       "1024"
        vulkan_batch_size                             "1000"

        // ===== Рендер =====
        r_citadel_antialiasing                        "0"  // без сглаживания
        vis_sunlight_enable                           "0"
        r_citadel_clip_sphere_min_opacity             "0"
        r_frame_sync_enable                           "0"
        r_nearz                                       "12"
        r_skip_precache_validation_check              "1"
        r_renderdoc_auto_shader_pdbs                  "0"
        mat_set_shader_quality                        "0"
        cpu_level                                     "0"
        gpu_level                                     "0"
        gpu_mem_level                                 "0"
        mem_level                                     "1"
        citadel_video_preset                          "0"

        // ===== Движок =====
        // fps_max тут нет, лимит FPS ставится в настройках игры
        fps_max_tools                                 "60"
        engine_no_focus_sleep                         "10"  // не тормозит, когда свернул игру; хочешь, чтобы в фоне грузила меньше, поставь 10
        engine_low_latency_sleep_after_client_tick    "1"
        engine_max_ticks_to_simulate                  "2"
        citadel_use_pvs_for_players                   "1"
        cl_smoothtime                                 "0.01"

        // ===== Интерфейс =====
        panorama_disable_blur                         "1"  // без размытия в меню
        panorama_disable_box_shadow                   "1"
        panorama_allow_transitions                    "0"  // без анимаций переходов в меню
        panorama_comp_layer_lru_lifetime              "4"  // меню и лавка открываются быстрее со второго раза
        panorama_render_target_cache_max_size         "67108864"
        citadel_in_world_item_panel_dpi               "0.5"
        citadel_hud_objective_health_idle_timeout     "0"
        citadel_damage_text_lifetime_new              "0.1"  // цифры урона висят 0.1 с
        citadel_damage_offscreen_indicator_disabled   "1"

        // ===== Обводка и подсветка =====
        citadel_trooper_outline_enabled               "true"
        citadel_trooper_glow_disabled                 "false"
        citadel_boss_glow_disabled                    "1"  // без свечения боссов
        cl_glow_brightness                            "1.5"  // обводка вокруг героев; 1 как у Valve, хочешь ярче, ставь 1.5

        // ===== Можно включить, если хочешь =====
        //r_light_sensitivity_mode                    "1"  // режим для светочувствительных; сначала попробуй в консоли (F7)
        //cloth_update                                "0"  // если стоят моды, убирающие плащи и одежду
        //mat_colorcorrection                         "0"  // если не используешь моды на цветокоррекцию
        //r_texture_pool_size                         "6000"  // меньше видеопамяти под текстуры
        //r_max_texture_pool_size                     "6000"

        // ===== Оптимизация процессора и потоков (Убирает фризы) =====
        r_threaded_particles                          "1"  // обработка частиц на разных ядрах CPU
        cl_threaded_bone_setup                        "1"  // параллельный расчет скелетной анимации героев
        anim_allow_asynchronous_animation             "1"  // асинхронные анимации, снижает нагрузку на главное ядро
        host_thread_mode                              "2"  // принудительно задействует все ядра процессора
        g_ragdoll_maxcount_per_frame                  "0"  // жесткое ограничение на расчет физики тел за кадр

        // ===== Память, стриминг и кэш шейдеров =====
        r_shader_execute_only_pre_compiled            "1"  // запрет компиляции тяжелых шейдеров «на лету» во время матча
        r_gpu_memory_management_mode                  "1"  // агрессивное управление видеопамятью
        mat_disable_d3d9_vertex_cache                 "1"  // очистка старого кэша вершинных шейдеров
        r_texture_stream_pool_full_trigger            "95" // оптимизация порога очистки текстурного пула

        // ===== Дополнительное сжатие интерфейса =====
        panorama_max_render_target_size               "1024" // ограничивает разрешение рендера элементов UI
        panorama_experimental_blur_optimization       "1"  // если где-то проскочит блюр, он будет максимально легковесным
    }
}
